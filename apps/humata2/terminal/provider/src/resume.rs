//! Pure explicit-session recovery planning. No processes, files or credentials.
use std::collections::BTreeMap;

pub const CLAUDE_RESUME_V1: &str = "claude-cli-resume-v1";
pub const CODEX_RESUME_V1: &str = "codex-cli-resume-v1";
pub const COPILOT_RESUME_V1: &str = "copilot-cli-resume-v1";

/// Trusted launcher policy, deliberately separate from persisted hook metadata.
/// Arguments are explicitly configured for recovery, never copied from a failed
/// launch's positional prompt or conversation selector.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ResumePolicy {
    #[default]
    Disabled,
    Enabled {
        adapter: String,
        provider: String,
        args: Vec<String>,
    },
}
impl<'de> serde::Deserialize<'de> for ResumePolicy {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // An internally tagged unit variant silently skips extra fields even
        // with deny_unknown_fields. An empty struct variant enforces the same
        // strict contract for Disabled as for Enabled, preserving the public API.
        #[derive(serde::Deserialize)]
        #[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
        enum Input {
            Disabled {},
            Enabled {
                adapter: String,
                provider: String,
                args: Vec<String>,
            },
        }
        Ok(
            match <Input as serde::Deserialize>::deserialize(deserializer)? {
                Input::Disabled {} => Self::Disabled,
                Input::Enabled {
                    adapter,
                    provider,
                    args,
                } => Self::Enabled {
                    adapter,
                    provider,
                    args,
                },
            },
        )
    }
}

/// Metadata supplied by structured provider events and saved by the host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResumeIdentity {
    pub provider: String,
    pub session_id: String,
    pub working_directory: String,
}

/// Complete process argv, excluding the executable. The host supplies its
/// trusted program and the original endpoint; neither comes from hook payloads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResumePlan {
    pub provider: String,
    pub session_id: String,
    pub working_directory: String,
    pub args: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ResumeError {
    #[error(
        "Automatic provider recovery is disabled. Explicitly enable a trusted resume policy before starting another process."
    )]
    Disabled,
    #[error(
        "Resume adapter {adapter} is not registered. Configure a verified versioned adapter; preserve the previous terminal instead of starting a fresh conversation."
    )]
    UnknownAdapter { adapter: String },
    #[error(
        "Resume adapter {adapter} is already registered. Use another versioned key; the existing adapter was preserved."
    )]
    DuplicateAdapter { adapter: String },
    #[error(
        "Saved provider identity does not match the trusted recovery policy. Bind the matching provider before resuming; no command was planned."
    )]
    ProviderMismatch,
    #[error(
        "Resume field {field} is invalid: {reason}. Correct the explicit recovery metadata before retrying."
    )]
    Field {
        field: &'static str,
        reason: &'static str,
    },
    #[error(
        "Resume field {field} needs {actual} bytes/items, above {maximum}. Reduce trusted recovery metadata before retrying."
    )]
    Limit {
        field: &'static str,
        actual: usize,
        maximum: usize,
    },
    #[error(
        "Resume adapter {adapter} requires a complete UUID conversation identifier. Supply the ID from a structured session event, never a name, prefix, path or latest-session selector."
    )]
    SessionId { adapter: String },
    #[error(
        "Recovery argument {index} is not supported by adapter {adapter}: {reason}. Configure resume-only options or register a compatible adapter; do not replay the original launch prompt."
    )]
    Argument {
        adapter: String,
        index: usize,
        reason: &'static str,
    },
}

/// Custom adapters are trusted launcher code. They must select the supplied
/// exact session, never continue/latest, fork, or recreate a missing session.
/// The registry validates identity and argv bounds around every adapter.
pub trait ResumeAdapter: Send + Sync {
    fn key(&self) -> &str;
    fn args(
        &self,
        identity: &ResumeIdentity,
        trusted_args: &[String],
    ) -> Result<Vec<String>, ResumeError>;
}

pub struct ResumeRegistry {
    adapters: BTreeMap<String, Box<dyn ResumeAdapter>>,
}
impl Default for ResumeRegistry {
    fn default() -> Self {
        let adapters = [Kind::Claude, Kind::Codex, Kind::Copilot]
            .into_iter()
            .map(|kind| {
                let adapter: Box<dyn ResumeAdapter> = Box::new(Builtin(kind));
                (adapter.key().to_owned(), adapter)
            })
            .collect();
        Self { adapters }
    }
}
impl ResumeRegistry {
    /// Validate launcher-owned policy fields before any process is started.
    /// Adapter-specific arguments are validated with the exact saved identity
    /// when planning; custom adapters may require their own identifier shape.
    pub fn validate_policy(&self, policy: &ResumePolicy) -> Result<(), ResumeError> {
        let ResumePolicy::Enabled {
            adapter,
            provider,
            args,
        } = policy
        else {
            return Ok(());
        };
        field(adapter, "adapter", 128)?;
        field(provider, "provider", 64)?;
        argv(args)?;
        if !self.adapters.contains_key(adapter) {
            return Err(ResumeError::UnknownAdapter {
                adapter: adapter.clone(),
            });
        }
        Ok(())
    }
    pub fn register(&mut self, adapter: Box<dyn ResumeAdapter>) -> Result<(), ResumeError> {
        field(adapter.key(), "adapter", 128)?;
        let key = adapter.key().to_owned();
        if self.adapters.contains_key(&key) {
            return Err(ResumeError::DuplicateAdapter { adapter: key });
        }
        self.adapters.insert(key, adapter);
        Ok(())
    }
    /// Planning performs no I/O and grants no permission to spawn. The host
    /// must independently establish process death, retain historical state and
    /// apply this explicit policy; a failed SSH dial is not death evidence.
    pub fn plan(
        &self,
        policy: &ResumePolicy,
        identity: &ResumeIdentity,
    ) -> Result<ResumePlan, ResumeError> {
        let ResumePolicy::Enabled {
            adapter,
            provider,
            args,
        } = policy
        else {
            return Err(ResumeError::Disabled);
        };
        field(adapter, "adapter", 128)?;
        field(provider, "provider", 64)?;
        field(&identity.provider, "provider", 64)?;
        field(&identity.session_id, "session_id", 512)?;
        field(&identity.working_directory, "working_directory", 4096)?;
        if provider != &identity.provider {
            return Err(ResumeError::ProviderMismatch);
        }
        argv(args)?;
        let planner = self
            .adapters
            .get(adapter)
            .ok_or_else(|| ResumeError::UnknownAdapter {
                adapter: adapter.clone(),
            })?;
        let planned = planner.args(identity, args)?;
        argv(&planned)?;
        if planned.is_empty() {
            return Err(ResumeError::Field {
                field: "args",
                reason: "the adapter returned no recovery arguments",
            });
        }
        Ok(ResumePlan {
            provider: identity.provider.clone(),
            session_id: identity.session_id.clone(),
            working_directory: identity.working_directory.clone(),
            args: planned,
        })
    }
}
fn field(value: &str, name: &'static str, maximum: usize) -> Result<(), ResumeError> {
    if value.len() > maximum {
        return Err(ResumeError::Limit {
            field: name,
            actual: value.len(),
            maximum,
        });
    }
    if value.trim().is_empty() || value.chars().any(char::is_control) {
        return Err(ResumeError::Field {
            field: name,
            reason: "expected nonempty text without control characters",
        });
    }
    Ok(())
}
fn argv(args: &[String]) -> Result<(), ResumeError> {
    if args.len() > 128 {
        return Err(ResumeError::Limit {
            field: "args",
            actual: args.len(),
            maximum: 128,
        });
    }
    let mut total = 0usize;
    for arg in args {
        if arg.len() > 16384 {
            return Err(ResumeError::Limit {
                field: "argument",
                actual: arg.len(),
                maximum: 16384,
            });
        }
        if arg.contains('\0') {
            return Err(ResumeError::Field {
                field: "argument",
                reason: "arguments must not contain NUL",
            });
        }
        total += arg.len();
    }
    if total > 65536 {
        return Err(ResumeError::Limit {
            field: "args_bytes",
            actual: total,
            maximum: 65536,
        });
    }
    Ok(())
}
#[derive(Clone, Copy)]
enum Kind {
    Claude,
    Codex,
    Copilot,
}
struct Builtin(Kind);
impl ResumeAdapter for Builtin {
    fn key(&self) -> &str {
        match self.0 {
            Kind::Claude => CLAUDE_RESUME_V1,
            Kind::Codex => CODEX_RESUME_V1,
            Kind::Copilot => COPILOT_RESUME_V1,
        }
    }
    fn args(
        &self,
        identity: &ResumeIdentity,
        trusted_args: &[String],
    ) -> Result<Vec<String>, ResumeError> {
        let id = identity.session_id.as_bytes();
        if id.len() != 36
            || id.iter().enumerate().any(|(i, b)| {
                if [8, 13, 18, 23].contains(&i) {
                    *b != b'-'
                } else {
                    !b.is_ascii_hexdigit()
                }
            })
        {
            return Err(ResumeError::SessionId {
                adapter: self.key().into(),
            });
        }
        // Narrow known resume options. Bare values could replay the original
        // prompt; unknown selection flags could silently select another session.
        // New CLI options belong in a new explicit adapter, not a guessed parse.
        let (values, flags): (&[&str], &[&str]) = match self.0 {
            Kind::Claude => (
                &[
                    "--model",
                    "--settings",
                    "--setting-sources",
                    "--mcp-config",
                    "--plugin-dir",
                    "--permission-mode",
                    "--add-dir",
                    "--effort",
                    "--agent",
                ],
                &["--verbose", "--no-chrome", "--disable-slash-commands"],
            ),
            Kind::Codex => (
                &[
                    "-c",
                    "--config",
                    "-m",
                    "--model",
                    "--enable",
                    "--disable",
                    "-s",
                    "--sandbox",
                    "-a",
                    "--ask-for-approval",
                    "--add-dir",
                    "-p",
                    "--profile",
                ],
                &["--no-alt-screen"],
            ),
            Kind::Copilot => (
                &[
                    "--model",
                    "--plugin-dir",
                    "--agent",
                    "--log-dir",
                    "--log-level",
                    "--context",
                ],
                &[
                    "--no-auto-update",
                    "--no-custom-instructions",
                    "--no-experimental",
                    "--no-mouse",
                    "--no-color",
                    "--screen-reader",
                ],
            ),
        };
        let mut i = 0;
        while i < trusted_args.len() {
            let argument = &trusted_args[i];
            let (key, inline) = argument
                .split_once('=')
                .map_or((argument.as_str(), false), |(key, _)| (key, true));
            if values.contains(&key) {
                if !inline {
                    if i + 1 == trusted_args.len() {
                        return Err(ResumeError::Argument {
                            adapter: self.key().into(),
                            index: i,
                            reason: "an option value is missing",
                        });
                    }
                    if trusted_args[i + 1].starts_with('-') {
                        return Err(ResumeError::Argument {
                            adapter: self.key().into(),
                            index: i + 1,
                            reason: "an option value cannot be another selector or flag; use an explicit inline value when appropriate",
                        });
                    }
                    i += 1;
                }
            } else if !inline && flags.contains(&key) {
            } else {
                return Err(ResumeError::Argument {
                    adapter: self.key().into(),
                    index: i,
                    reason: "a positional prompt, conflicting session selector or unknown option was supplied",
                });
            }
            i += 1;
        }
        let mut result = Vec::new();
        if matches!(self.0, Kind::Codex) {
            result.push("resume".into());
        }
        result.extend_from_slice(trusted_args);
        match self.0 {
            Kind::Claude => {
                result.push("--resume".into());
                result.push(identity.session_id.clone());
            }
            Kind::Codex => result.push(identity.session_id.clone()),
            Kind::Copilot => result.push(format!("--resume={}", identity.session_id)),
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn identity() -> ResumeIdentity {
        ResumeIdentity {
            provider: "configured-provider".into(),
            session_id: "019a83ae-6368-72f5-90bc-5799b82886a1".into(),
            working_directory: "/explicit/work".into(),
        }
    }
    fn policy(adapter: &str, args: &[&str]) -> ResumePolicy {
        ResumePolicy::Enabled {
            adapter: adapter.into(),
            provider: "configured-provider".into(),
            args: args.iter().map(|s| (*s).into()).collect(),
        }
    }
    #[test]
    fn default_policy_never_plans_a_replacement() {
        assert!(matches!(
            ResumeRegistry::default().plan(&ResumePolicy::default(), &identity()),
            Err(ResumeError::Disabled)
        ));
    }
    #[test]
    fn launcher_policy_json_is_explicit_and_cannot_supply_an_executable_or_endpoint() {
        let disabled: ResumePolicy =
            serde_json::from_value(serde_json::json!({"mode":"disabled"})).unwrap();
        assert_eq!(disabled, ResumePolicy::Disabled);
        let enabled = serde_json::json!({"mode":"enabled","adapter":CODEX_RESUME_V1,"provider":"configured-provider","args":[]});
        let decoded: ResumePolicy = serde_json::from_value(enabled.clone()).unwrap();
        assert_eq!(decoded, policy(CODEX_RESUME_V1, &[]));
        ResumeRegistry::default().validate_policy(&decoded).unwrap();
        for field in ["program", "executable", "endpoint"] {
            for mut value in [enabled.clone(), serde_json::json!({"mode":"disabled"})] {
                value[field] = serde_json::json!("must-not-be-accepted");
                assert!(serde_json::from_value::<ResumePolicy>(value).is_err());
            }
        }
        let unknown: ResumePolicy=serde_json::from_value(serde_json::json!({"mode":"enabled","adapter":"unknown-resume-version","provider":"configured-provider","args":[]})).unwrap();
        assert!(matches!(
            ResumeRegistry::default().validate_policy(&unknown),
            Err(ResumeError::UnknownAdapter { .. })
        ));
    }
    #[test]
    fn builtins_select_exact_saved_conversation_and_keep_resume_only_options() {
        let registry = ResumeRegistry::default();
        let id = identity();
        assert_eq!(
            registry
                .plan(&policy(CLAUDE_RESUME_V1, &["--setting-sources", ""]), &id)
                .unwrap()
                .args,
            vec!["--setting-sources", "", "--resume", &id.session_id]
        );
        assert_eq!(
            registry
                .plan(&policy(CLAUDE_RESUME_V1, &["--model", "sonnet"]), &id)
                .unwrap()
                .args,
            vec!["--model", "sonnet", "--resume", &id.session_id]
        );
        assert_eq!(
            registry
                .plan(&policy(CODEX_RESUME_V1, &["-c", "hooks={}"]), &id)
                .unwrap()
                .args,
            vec!["resume", "-c", "hooks={}", &id.session_id]
        );
        assert_eq!(
            registry
                .plan(&policy(COPILOT_RESUME_V1, &[]), &id)
                .unwrap()
                .args,
            vec![format!("--resume={}", id.session_id)]
        );
        assert_eq!(
            registry
                .plan(&policy(CODEX_RESUME_V1, &[]), &id)
                .unwrap()
                .working_directory,
            id.working_directory
        );
    }
    #[test]
    fn names_prefixes_paths_and_latest_selectors_are_not_session_ids() {
        for adapter in [CLAUDE_RESUME_V1, CODEX_RESUME_V1, COPILOT_RESUME_V1] {
            for value in [
                "latest",
                "--last",
                "task-name",
                "019a83a",
                "/tmp/session.jsonl",
                "",
            ] {
                let mut id = identity();
                id.session_id = value.into();
                assert!(
                    ResumeRegistry::default()
                        .plan(&policy(adapter, &[]), &id)
                        .is_err()
                );
            }
        }
    }
    #[test]
    fn conflicting_selection_and_replayed_prompts_fail_before_any_plan() {
        for adapter in [CLAUDE_RESUME_V1, CODEX_RESUME_V1, COPILOT_RESUME_V1] {
            for args in [
                vec!["--continue"],
                vec!["--resume=x"],
                vec!["--session-id", "x"],
                vec!["--fork-session"],
                vec!["original prompt"],
                vec!["--"],
                vec!["--model"],
                vec!["-C", "/another/project"],
                vec!["--cd=/another/project"],
            ] {
                assert!(matches!(
                    ResumeRegistry::default().plan(&policy(adapter, &args), &identity()),
                    Err(ResumeError::Argument { .. })
                ));
            }
        }
    }
    #[test]
    fn trusted_provider_identity_and_limits_apply_before_planning() {
        let mut id = identity();
        id.provider = "another".into();
        assert!(matches!(
            ResumeRegistry::default().plan(&policy(CODEX_RESUME_V1, &[]), &id),
            Err(ResumeError::ProviderMismatch)
        ));
        let mut id = identity();
        id.working_directory.push('\n');
        assert!(matches!(
            ResumeRegistry::default().plan(&policy(CODEX_RESUME_V1, &[]), &id),
            Err(ResumeError::Field { .. })
        ));
        let mut id = identity();
        id.session_id = "x".repeat(513);
        assert!(matches!(
            ResumeRegistry::default().plan(&policy(CODEX_RESUME_V1, &[]), &id),
            Err(ResumeError::Limit { .. })
        ));
    }
    struct Custom;
    impl ResumeAdapter for Custom {
        fn key(&self) -> &str {
            "custom-v1"
        }
        fn args(&self, id: &ResumeIdentity, _: &[String]) -> Result<Vec<String>, ResumeError> {
            Ok(vec!["--exact-session".into(), id.session_id.clone()])
        }
    }
    #[test]
    fn custom_opaque_ids_are_supported_without_replacing_existing_adapters() {
        let mut registry = ResumeRegistry::default();
        registry.register(Box::new(Custom)).unwrap();
        assert!(matches!(
            registry.register(Box::new(Custom)),
            Err(ResumeError::DuplicateAdapter { .. })
        ));
        let mut id = identity();
        id.session_id = "ses_explicit_opaque".into();
        assert_eq!(
            registry.plan(&policy("custom-v1", &[]), &id).unwrap().args,
            vec!["--exact-session", "ses_explicit_opaque"]
        );
        assert!(matches!(
            registry.plan(&policy("not-registered", &[]), &id),
            Err(ResumeError::UnknownAdapter { .. })
        ));
    }
}
