use super::*;
use serde_json::{Value, json};

fn context(schema: &str, event: Option<&str>) -> TrustedContext {
    TrustedContext {
        provider: "trusted-provider".into(),
        schema: schema.into(),
        hook_event: event.map(Into::into),
        state_paths: vec!["/explicit/provider-state".into()],
    }
}
fn normalize(
    schema: &str,
    event: Option<&str>,
    payload: Value,
) -> Result<Option<ProviderEvent>, Error> {
    Registry::default().normalize(
        &context(schema, event),
        &serde_json::to_vec(&payload).unwrap(),
    )
}
fn hook(event: &str) -> Value {
    json!({"hook_event_name":event,"session_id":"explicit-session","cwd":"/workspace","source":"resume"})
}

#[test]
fn claude_start_keeps_explicit_identity_and_trusted_binding_without_path_inference() {
    let mut payload = hook("SessionStart");
    payload["provider"] = json!("spoofed-provider");
    payload["state_paths"] = json!(["/untrusted/state"]);
    payload["transcript_path"] = json!({"not":"even a path"});
    payload["prompt"] = json!(["unread",{"text":"not normalized"}]);
    let event = normalize(CLAUDE_HOOKS_V1, None, payload).unwrap().unwrap();
    assert_eq!(event.provider, "trusted-provider");
    assert_eq!(event.schema, CLAUDE_HOOKS_V1);
    assert_eq!(event.session_id, "explicit-session");
    assert_eq!(event.cwd.as_deref(), Some("/workspace"));
    assert_eq!(
        event.signal,
        ProviderSignal::SessionStarted {
            source: Some("resume".into())
        }
    );
}

#[test]
fn notification_text_is_only_taken_from_the_documented_notification_event() {
    let mut payload = hook("Notification");
    payload["message"] = json!("Permission needed\nChoose an action");
    payload["title"] = json!("Permission");
    payload["notification_type"] = json!("permission_prompt");
    assert_eq!(
        normalize(CLAUDE_HOOKS_V1, None, payload)
            .unwrap()
            .unwrap()
            .signal,
        ProviderSignal::Notification {
            kind: "permission_prompt".into(),
            title: Some("Permission".into()),
            message: "Permission needed\nChoose an action".into()
        }
    );
    let mut stop = hook("Stop");
    stop["last_assistant_message"] = json!("ignored".repeat(5000));
    stop["message"] = json!({"ignored":"not a notification"});
    assert_eq!(
        normalize(CLAUDE_HOOKS_V1, None, stop)
            .unwrap()
            .unwrap()
            .signal,
        ProviderSignal::TurnFinished
    );
}

#[test]
fn codex_lifecycle_is_not_a_pty_exit_or_an_undocumented_notification() {
    for (name, signal) in [
        ("Stop", ProviderSignal::TurnFinished),
        ("Interrupt", ProviderSignal::Interrupted),
        ("SessionEnd", ProviderSignal::SessionEnded),
        ("UserPromptSubmit", ProviderSignal::TurnStarted),
    ] {
        assert_eq!(
            normalize(CODEX_HOOKS_V1, None, hook(name))
                .unwrap()
                .unwrap()
                .signal,
            signal
        );
    }
    assert_eq!(
        normalize(CODEX_HOOKS_V1, None, hook("SessionStart"))
            .unwrap()
            .unwrap()
            .session_id,
        "explicit-session"
    );
    assert!(
        normalize(CODEX_HOOKS_V1, None, hook("Notification"))
            .unwrap()
            .is_none()
    );
}

#[test]
fn copilot_camel_and_pascal_formats_are_explicit_and_equivalent() {
    let camel=normalize(COPILOT_CAMEL_HOOKS_V1,Some("sessionStart"),
        json!({"sessionId":"explicit-session","cwd":"/workspace","source":"resume","initialPrompt":{"unread":true}})).unwrap().unwrap();
    let pascal = normalize(COPILOT_VSCODE_HOOKS_V1, None, hook("SessionStart"))
        .unwrap()
        .unwrap();
    assert_eq!(camel.session_id, pascal.session_id);
    assert_eq!(camel.cwd, pascal.cwd);
    assert_eq!(camel.signal, pascal.signal);
    assert!(matches!(
        normalize(
            COPILOT_CAMEL_HOOKS_V1,
            None,
            json!({"sessionId":"s","cwd":"/w"})
        ),
        Err(Error::MissingField {
            field: "hook_event"
        })
    ));
    assert!(normalize(COPILOT_VSCODE_HOOKS_V1,None,json!({"hook_event_name":"SessionStart","sessionId":"wrong-format","cwd":"/w","source":"startup"})).is_err());
}

#[test]
fn copilot_notification_accepts_its_documented_mixed_case_event_field() {
    let payload = json!({"sessionId":"copilot-session","cwd":"/workspace","hook_event_name":"Notification","notification_type":"agent_idle","message":"Ready","title":"Status"});
    assert!(matches!(
        normalize(COPILOT_CAMEL_HOOKS_V1, Some("notification"), payload)
            .unwrap()
            .unwrap()
            .signal,
        ProviderSignal::Notification { .. }
    ));
    let mut mismatch = hook("Stop");
    mismatch["sessionId"] = json!("s");
    assert!(matches!(
        normalize(COPILOT_CAMEL_HOOKS_V1, Some("sessionStart"), mismatch),
        Err(Error::EventMismatch)
    ));
}

#[test]
fn opencode_v2_uses_data_session_id_without_guessing_paths_or_legacy_properties() {
    let event=normalize(OPENCODE_EVENTS_V2,None,json!({"type":"session.created","data":{"sessionID":"ses_explicit","location":{"opaque":"reference"}},"properties":{"info":{"id":"old-id","directory":"/do-not-infer"}}})).unwrap().unwrap();
    assert_eq!(event.session_id, "ses_explicit");
    assert_eq!(event.cwd, None);
    assert!(matches!(
        event.signal,
        ProviderSignal::SessionStarted { .. }
    ));
    assert_eq!(
        normalize(
            OPENCODE_EVENTS_V2,
            None,
            json!({"type":"session.idle","data":{"sessionID":"ses_explicit"}})
        )
        .unwrap()
        .unwrap()
        .signal,
        ProviderSignal::TurnFinished
    );
    assert!(
        normalize(
            OPENCODE_EVENTS_V2,
            None,
            json!({"type":"session.created","properties":{"info":{"id":"legacy"}}})
        )
        .is_err()
    );
}

#[test]
fn root_adapters_ignore_subagent_and_child_session_events() {
    assert!(
        normalize(CLAUDE_HOOKS_V1, None, hook("SubagentStop"))
            .unwrap()
            .is_none()
    );
    let mut child = hook("SessionStart");
    child["agent_id"] = json!("child-agent");
    assert!(normalize(CLAUDE_HOOKS_V1, None, child).unwrap().is_none());
    assert!(
        normalize(
            OPENCODE_EVENTS_V2,
            None,
            json!({"type":"session.created","data":{"sessionID":"child","parentID":"parent"}})
        )
        .unwrap()
        .is_none()
    );
}

#[test]
fn unknown_event_kinds_skip_unused_fields_but_complete_json_is_still_required() {
    assert!(
        normalize(
            CLAUDE_HOOKS_V1,
            None,
            json!({"hook_event_name":"FutureEvent","session_id":{"ignored":true}})
        )
        .unwrap()
        .is_none()
    );
    assert!(
        normalize(
            OPENCODE_EVENTS_V2,
            None,
            json!({"type":"future.event","data":["ignored"]})
        )
        .unwrap()
        .is_none()
    );
    assert!(matches!(
        Registry::default().normalize(
            &context(CLAUDE_HOOKS_V1, None),
            b"{\"hook_event_name\":\"FutureEvent\"} {}"
        ),
        Err(Error::Json { .. })
    ));
}

#[test]
fn oversized_payload_is_rejected_before_attempting_to_parse_it() {
    let bytes = vec![b' '; MAX_PAYLOAD_BYTES + 1];
    assert!(matches!(
        Registry::default().normalize(&context(CLAUDE_HOOKS_V1, None), &bytes),
        Err(Error::PayloadLimit { .. })
    ));
}

#[test]
fn recognized_events_require_usable_bounded_session_and_working_directory_fields() {
    for (field, value) in [
        ("session_id", json!("")),
        ("session_id", json!("s".repeat(513))),
        ("cwd", json!("/".repeat(4097))),
        ("session_id", json!("line\nbreak")),
    ] {
        let mut payload = hook("Stop");
        payload[field] = value;
        assert!(normalize(CLAUDE_HOOKS_V1, None, payload).is_err());
    }
    assert!(matches!(
        normalize(
            CLAUDE_HOOKS_V1,
            None,
            json!({"hook_event_name":"Stop","cwd":"/w"})
        ),
        Err(Error::MissingField {
            field: "session_id"
        })
    ));
    assert!(matches!(
        normalize(
            CLAUDE_HOOKS_V1,
            None,
            json!({"hook_event_name":"Stop","session_id":7,"cwd":"/w"})
        ),
        Err(Error::Json { .. })
    ));
}

#[test]
fn notification_and_trusted_context_limits_are_enforced() {
    for (field, size) in [
        ("message", 8193),
        ("title", 513),
        ("notification_type", 129),
    ] {
        let mut payload = hook("Notification");
        payload["message"] = json!("Ready");
        payload["notification_type"] = json!("idle");
        payload[field] = json!("x".repeat(size));
        assert!(matches!(
            normalize(CLAUDE_HOOKS_V1, None, payload),
            Err(Error::FieldLimit { .. })
        ));
    }
    let registry = Registry::default();
    let mut binding = context(CLAUDE_HOOKS_V1, None);
    binding.state_paths = vec!["/state".into(); 17];
    assert!(matches!(
        registry.normalize(&binding, b"{}"),
        Err(Error::FieldLimit {
            field: "state_paths",
            ..
        })
    ));
    binding = context(CLAUDE_HOOKS_V1, None);
    binding.provider = "x".repeat(65);
    assert!(matches!(
        registry.normalize(&binding, b"{}"),
        Err(Error::FieldLimit {
            field: "provider",
            ..
        })
    ));
}

struct Custom {
    schema: &'static str,
    message: usize,
}
impl Adapter for Custom {
    fn schema(&self) -> &str {
        self.schema
    }
    fn normalize(&self, _: &[u8], _: Option<&str>) -> Result<Option<NormalizedEvent>, Error> {
        Ok(Some(NormalizedEvent {
            session_id: "custom-session".into(),
            cwd: None,
            signal: ProviderSignal::Notification {
                kind: "custom".into(),
                title: None,
                message: "x".repeat(self.message),
            },
        }))
    }
}

#[test]
fn custom_adapters_keep_the_same_bounds_and_cannot_replace_existing_versions() {
    let mut registry = Registry::default();
    registry
        .register(Box::new(Custom {
            schema: "custom-v1",
            message: 3,
        }))
        .unwrap();
    let event = registry
        .normalize(&context("custom-v1", None), b"{}")
        .unwrap()
        .unwrap();
    assert_eq!(event.provider, "trusted-provider");
    assert_eq!(event.session_id, "custom-session");
    assert!(matches!(
        registry.register(Box::new(Custom {
            schema: CLAUDE_HOOKS_V1,
            message: 1
        })),
        Err(Error::DuplicateSchema { .. })
    ));
    assert!(matches!(
        registry
            .normalize(
                &context(CLAUDE_HOOKS_V1, None),
                &serde_json::to_vec(&hook("Stop")).unwrap()
            )
            .unwrap()
            .unwrap()
            .signal,
        ProviderSignal::TurnFinished
    ));
    registry
        .register(Box::new(Custom {
            schema: "oversized-v1",
            message: 8193,
        }))
        .unwrap();
    assert!(matches!(
        registry.normalize(&context("oversized-v1", None), b"{}"),
        Err(Error::FieldLimit {
            field: "message",
            ..
        })
    ));
    assert!(matches!(
        registry.normalize(&context("unregistered-v1", None), b"{}"),
        Err(Error::UnknownSchema { .. })
    ));
}

#[test]
fn native_events_must_match_the_registered_binding_as_well_as_field_bounds() {
    let binding = context(CODEX_HOOKS_V1, None);
    let mut event = normalize(CODEX_HOOKS_V1, None, hook("Stop"))
        .unwrap()
        .unwrap();
    validate_event(&binding, &event).unwrap();
    event.provider = "other".into();
    assert!(matches!(
        validate_event(&binding, &event),
        Err(Error::BindingIdentity)
    ));
    event.provider = binding.provider.clone();
    event.session_id.clear();
    assert!(matches!(
        validate_event(&binding, &event),
        Err(Error::InvalidField {
            field: "session_id",
            ..
        })
    ));
}
