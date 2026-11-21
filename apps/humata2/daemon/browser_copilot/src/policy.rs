//! Agent Inbox Permission Policy for Humata 2 Native Browser Co-Pilot.
//!
//! Strictly scoped to `claude`, `codex`, `copilot`, and `agy` and strictly
//! read-only (`ListTabs`, `ReadTab`, `Screenshot`):
//! - All supported browser actions are **Read actions** and may be granted
//!   `AllowOnce` or a 30-minute session grant (`AllowFor30Min`, 1,800,000 ms).
//! - Execute / write / mutate actions (`click`, `type`, `navigate`, `submit`,
//!   `highlight_guide`, scripting) do not exist in `BrowserAction` and are
//!   rejected at deserialization.

use std::collections::HashMap;
use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::protocol::BrowserAction;

/// Duration of a `Read` session grant (30 minutes in milliseconds).
pub const READ_GRANT_TTL_MS: u64 = 30 * 60 * 1000;

/// Agents permitted to request browser co-pilot access.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SupportedAgent {
    Claude,
    Codex,
    Copilot,
    Agy,
}

impl SupportedAgent {
    /// Parses an agent identifier, rejecting any agent outside `{claude, codex, copilot, agy}`.
    pub fn parse(raw: &str) -> Result<Self, PolicyError> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "claude" => Ok(SupportedAgent::Claude),
            "codex" => Ok(SupportedAgent::Codex),
            "copilot" => Ok(SupportedAgent::Copilot),
            "agy" => Ok(SupportedAgent::Agy),
            _ => Err(PolicyError::UnsupportedAgent(raw.to_string())),
        }
    }

    /// Canonical lowercase agent identifier.
    pub const fn as_str(self) -> &'static str {
        match self {
            SupportedAgent::Claude => "claude",
            SupportedAgent::Codex => "codex",
            SupportedAgent::Copilot => "copilot",
            SupportedAgent::Agy => "agy",
        }
    }
}

impl fmt::Display for SupportedAgent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Risk classification for browser actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionClass {
    Read,
    Write,
}

impl ActionClass {
    /// Classifies a `BrowserAction`. Every supported action is strictly `Read`.
    pub const fn classify(action: &BrowserAction) -> Self {
        match action {
            BrowserAction::ListTabs
            | BrowserAction::ReadTab { .. }
            | BrowserAction::Screenshot { .. } => ActionClass::Read,
        }
    }
}

/// User decision returned from the Agent Inbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    AllowOnce,
    AllowFor30Min,
    Deny,
}

/// Policy errors returned by `PermissionGate`.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum PolicyError {
    #[error("unsupported agent `{0}`; browser co-pilot is restricted to claude, codex, copilot, agy")]
    UnsupportedAgent(String),
    #[error("write browser actions require per-action approval and never accept AllowFor30Min")]
    WriteRequiresPerActionApproval,
    #[error("unknown or already-resolved inbox ask id `{0}`")]
    UnknownAskId(String),
}

/// Structured approval prompt emitted to the Agent Inbox.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserInboxAsk {
    pub ask_id: String,
    pub agent: SupportedAgent,
    pub action_class: ActionClass,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_preview: Option<String>,
    pub action_summary: String,
    pub allowed_decisions: Vec<Decision>,
    pub created_at_ms: u64,
}

/// Result of checking a browser action against `PermissionGate`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateCheck {
    /// Action is permitted immediately via an active `Read` session grant.
    Allowed,
    /// Action requires user approval via the Agent Inbox.
    NeedsApproval(BrowserInboxAsk),
}

/// Outcome of resolving a pending `BrowserInboxAsk`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalOutcome {
    /// Approved for this single action only; no session grant was cached.
    ApprovedOnce,
    /// Approved and cached a 30-minute `Read` session grant expiring at `expires_at_ms`.
    ApprovedWithSessionGrant { expires_at_ms: u64 },
    /// Explicitly denied by the user.
    Denied,
}

#[derive(Debug, Clone)]
struct PendingAsk {
    agent: SupportedAgent,
    action_class: ActionClass,
}

/// Stateful Agent Inbox permission gate for read-only browser co-pilot actions.
#[derive(Debug, Default)]
pub struct PermissionGate {
    /// Active 30-minute `Read` session grant expiration timestamps (`expires_at_ms`) per agent.
    read_grants: HashMap<SupportedAgent, u64>,
    pending_asks: HashMap<String, PendingAsk>,
    next_ask_seq: u64,
}

impl PermissionGate {
    pub fn new() -> Self {
        Self::default()
    }

    /// Evaluates whether `agent_name` may execute `action` at timestamp `now_ms`.
    pub fn check(
        &mut self,
        agent_name: &str,
        action: &BrowserAction,
        now_ms: u64,
    ) -> Result<GateCheck, PolicyError> {
        let agent = SupportedAgent::parse(agent_name)?;
        let action_class = ActionClass::classify(action);

        if action_class == ActionClass::Read {
            if let Some(&expires_at_ms) = self.read_grants.get(&agent) {
                if now_ms < expires_at_ms {
                    return Ok(GateCheck::Allowed);
                }
                self.read_grants.remove(&agent);
            }
        }

        self.next_ask_seq += 1;
        let ask_id = format!("browser-ask-{}", self.next_ask_seq);
        let allowed_decisions = match action_class {
            ActionClass::Read => vec![Decision::AllowOnce, Decision::AllowFor30Min, Decision::Deny],
            ActionClass::Write => vec![Decision::AllowOnce, Decision::Deny],
        };

        let ask = BrowserInboxAsk {
            ask_id: ask_id.clone(),
            agent,
            action_class,
            target_url: action.target_url().map(str::to_string),
            target_preview: action.target_preview(),
            action_summary: action.action_summary(),
            allowed_decisions,
            created_at_ms: now_ms,
        };

        self.pending_asks.insert(
            ask_id,
            PendingAsk {
                agent,
                action_class,
            },
        );

        Ok(GateCheck::NeedsApproval(ask))
    }

    /// Resolves a pending `BrowserInboxAsk` with the user's `decision`.
    pub fn resolve_ask(
        &mut self,
        ask_id: &str,
        decision: Decision,
        now_ms: u64,
    ) -> Result<ApprovalOutcome, PolicyError> {
        let pending = self
            .pending_asks
            .remove(ask_id)
            .ok_or_else(|| PolicyError::UnknownAskId(ask_id.to_string()))?;

        match (pending.action_class, decision) {
            (_, Decision::Deny) => Ok(ApprovalOutcome::Denied),
            (ActionClass::Read | ActionClass::Write, Decision::AllowOnce) => {
                Ok(ApprovalOutcome::ApprovedOnce)
            }
            (ActionClass::Read, Decision::AllowFor30Min) => {
                let expires_at_ms = now_ms.saturating_add(READ_GRANT_TTL_MS);
                self.read_grants.insert(pending.agent, expires_at_ms);
                Ok(ApprovalOutcome::ApprovedWithSessionGrant { expires_at_ms })
            }
            (ActionClass::Write, Decision::AllowFor30Min) => {
                self.pending_asks.insert(ask_id.to_string(), pending);
                Err(PolicyError::WriteRequiresPerActionApproval)
            }
        }
    }

    /// Explicitly revokes an agent's active 30-minute `Read` session grant.
    pub fn revoke_read_grant(&mut self, agent: SupportedAgent) {
        self.read_grants.remove(&agent);
    }

    /// Returns the expiration timestamp of an active `Read` grant for `agent` at `now_ms`.
    pub fn read_grant_expires_at(&self, agent: SupportedAgent, now_ms: u64) -> Option<u64> {
        self.read_grants
            .get(&agent)
            .copied()
            .filter(|&exp| now_ms < exp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strictly_scoped_to_claude_codex_copilot_agy() {
        assert_eq!(SupportedAgent::parse("claude"), Ok(SupportedAgent::Claude));
        assert_eq!(SupportedAgent::parse("CODEX"), Ok(SupportedAgent::Codex));
        assert_eq!(
            SupportedAgent::parse(" copilot "),
            Ok(SupportedAgent::Copilot)
        );
        assert_eq!(SupportedAgent::parse("Agy"), Ok(SupportedAgent::Agy));

        let mut gate = PermissionGate::new();
        for rejected in ["cursor", "opencode", "aider", "windsurf", ""] {
            assert_eq!(
                gate.check(rejected, &BrowserAction::ListTabs, 1_000),
                Err(PolicyError::UnsupportedAgent(rejected.to_string()))
            );
        }
    }

    #[test]
    fn read_30_min_grant_covers_list_read_and_screenshot_and_expires() {
        let mut gate = PermissionGate::new();
        let t0 = 10_000;

        let ask = match gate.check("claude", &BrowserAction::ListTabs, t0).unwrap() {
            GateCheck::NeedsApproval(ask) => ask,
            GateCheck::Allowed => panic!("expected approval ask"),
        };
        assert_eq!(ask.action_class, ActionClass::Read);
        assert_eq!(
            ask.allowed_decisions,
            vec![Decision::AllowOnce, Decision::AllowFor30Min, Decision::Deny]
        );

        let outcome = gate
            .resolve_ask(&ask.ask_id, Decision::AllowFor30Min, t0)
            .unwrap();
        assert_eq!(
            outcome,
            ApprovalOutcome::ApprovedWithSessionGrant {
                expires_at_ms: t0 + READ_GRANT_TTL_MS
            }
        );

        // 29m59s later -> ReadTab and Screenshot both allowed without asking.
        let t_29m59s = t0 + (29 * 60 + 59) * 1_000;
        assert_eq!(
            gate.check(
                "claude",
                &BrowserAction::ReadTab {
                    tab_id: None,
                    max_chars: None
                },
                t_29m59s
            )
            .unwrap(),
            GateCheck::Allowed
        );
        assert_eq!(
            gate.check("claude", &BrowserAction::Screenshot { tab_id: Some(1) }, t_29m59s)
                .unwrap(),
            GateCheck::Allowed
        );

        // Other supported agents do not inherit claude's grant.
        assert!(matches!(
            gate.check("codex", &BrowserAction::ListTabs, t_29m59s)
                .unwrap(),
            GateCheck::NeedsApproval(_)
        ));

        // 30m01s later -> expired, must ask again.
        let t_30m01s = t0 + (30 * 60 + 1) * 1_000;
        assert!(matches!(
            gate.check("claude", &BrowserAction::ListTabs, t_30m01s)
                .unwrap(),
            GateCheck::NeedsApproval(_)
        ));
    }

    #[test]
    fn read_allow_once_and_deny_do_not_cache_grant() {
        let mut gate = PermissionGate::new();
        let t0 = 5_000;

        let ask1 = match gate.check("copilot", &BrowserAction::ListTabs, t0).unwrap() {
            GateCheck::NeedsApproval(ask) => ask,
            GateCheck::Allowed => panic!("expected ask"),
        };
        assert_eq!(
            gate.resolve_ask(&ask1.ask_id, Decision::AllowOnce, t0)
                .unwrap(),
            ApprovalOutcome::ApprovedOnce
        );

        // Next read immediately asks again.
        let ask2 = match gate
            .check("copilot", &BrowserAction::Screenshot { tab_id: None }, t0 + 100)
            .unwrap()
        {
            GateCheck::NeedsApproval(ask) => ask,
            GateCheck::Allowed => panic!("AllowOnce must not cache a session grant"),
        };
        assert_eq!(
            gate.resolve_ask(&ask2.ask_id, Decision::Deny, t0 + 100)
                .unwrap(),
            ApprovalOutcome::Denied
        );
        assert!(gate.read_grant_expires_at(SupportedAgent::Copilot, t0 + 200).is_none());
    }

    #[test]
    fn all_supported_actions_classify_as_read() {
        let actions = [
            BrowserAction::ListTabs,
            BrowserAction::ReadTab {
                tab_id: None,
                max_chars: None,
            },
            BrowserAction::Screenshot { tab_id: Some(1) },
        ];
        for action in &actions {
            assert_eq!(ActionClass::classify(action), ActionClass::Read);
        }
    }
}
