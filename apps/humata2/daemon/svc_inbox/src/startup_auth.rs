//! ATM-54 SESSIONBOOT: deterministic TUI startup authorization and prompt
//! handling.
//!
//! Provides bounded, deterministically jittered startup polling, explicit
//! owner-consent scope enforcement, TUI prompt classification and key-mapping
//! selection, smart-agent approval boundary summaries, and a structured audit
//! trail.

use std::path::{Path, PathBuf};
use std::time::Duration;

use humata2_daemon_runtime::screen_question::{self, PromptKind};
use sha2::{Digest, Sha256};
use thiserror::Error;

/// Explicit errors returned by startup authorization operations.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum StartupAuthError {
    #[error("failed to read startup terminal screen: {detail}")]
    ScreenRead { detail: String },
    #[error("failed to send startup keys to terminal: {detail}")]
    KeySend { detail: String },
    #[error("invalid startup authorization configuration: {detail}")]
    InvalidConfig { detail: String },
}

/// Bounded startup polling and deterministic jitter configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupConfig {
    pub initial_wait: Duration,
    pub base_retry_delay: Duration,
    pub max_retry_delay: Duration,
    pub max_total_wait: Duration,
    pub max_attempts: usize,
    pub jitter_ratio_bps: u16,
    pub seed: u64,
}

impl Default for StartupConfig {
    fn default() -> Self {
        Self {
            initial_wait: Duration::from_millis(250),
            base_retry_delay: Duration::from_millis(200),
            max_retry_delay: Duration::from_millis(2_000),
            max_total_wait: Duration::from_millis(15_000),
            max_attempts: 12,
            jitter_ratio_bps: 2_000,
            seed: 0,
        }
    }
}

impl StartupConfig {
    /// Validates that all bounds and jitter settings are well-formed.
    pub fn validate(&self) -> Result<(), StartupAuthError> {
        if self.max_attempts == 0 {
            return Err(StartupAuthError::InvalidConfig {
                detail: "max_attempts must be greater than 0".to_owned(),
            });
        }
        if self.base_retry_delay > self.max_retry_delay {
            return Err(StartupAuthError::InvalidConfig {
                detail: format!(
                    "base_retry_delay ({:?}) exceeds max_retry_delay ({:?})",
                    self.base_retry_delay, self.max_retry_delay
                ),
            });
        }
        if self.jitter_ratio_bps > 10_000 {
            return Err(StartupAuthError::InvalidConfig {
                detail: format!(
                    "jitter_ratio_bps ({}) must be <= 10000",
                    self.jitter_ratio_bps
                ),
            });
        }
        if self.initial_wait > self.max_total_wait {
            return Err(StartupAuthError::InvalidConfig {
                detail: format!(
                    "initial_wait ({:?}) exceeds max_total_wait ({:?})",
                    self.initial_wait, self.max_total_wait
                ),
            });
        }
        Ok(())
    }
}

/// Deterministic exponential backoff schedule with bounded ±jitter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupSchedule {
    config: StartupConfig,
}

impl StartupSchedule {
    /// Creates a validated schedule from `config`.
    pub fn new(config: StartupConfig) -> Result<Self, StartupAuthError> {
        config.validate()?;
        Ok(Self { config })
    }

    /// Returns the underlying configuration.
    pub fn config(&self) -> &StartupConfig {
        &self.config
    }

    /// Computes the deterministic jittered delay for `attempt` (0-indexed),
    /// clamped to `[0, max_retry_delay]`.
    pub fn delay_for_attempt(&self, attempt: usize) -> Duration {
        let base_ms = self.config.base_retry_delay.as_millis();
        let max_ms = self.config.max_retry_delay.as_millis();
        if base_ms == 0 || max_ms == 0 {
            return Duration::ZERO;
        }
        let shift = (attempt as u32).min(30);
        let exp_ms = base_ms.saturating_mul(1u128 << shift).min(max_ms);
        let max_jitter_ms = exp_ms
            .saturating_mul(u128::from(self.config.jitter_ratio_bps))
            / 10_000;
        if max_jitter_ms == 0 {
            return Duration::from_millis(exp_ms.min(u128::from(u64::MAX)) as u64);
        }
        let hash = splitmix64(
            self.config
                .seed
                .wrapping_add((attempt as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)),
        );
        let span = max_jitter_ms.saturating_mul(2).saturating_add(1);
        let offset = u128::from(hash) % span;
        let jittered = exp_ms
            .saturating_add(offset)
            .saturating_sub(max_jitter_ms)
            .min(max_ms);
        Duration::from_millis(jittered.min(u128::from(u64::MAX)) as u64)
    }

    /// Returns the next bounded delay respecting `max_attempts` and
    /// `max_total_wait`, or `None` when the budget is exhausted.
    pub fn bounded_delay(&self, attempt: usize, elapsed: Duration) -> Option<Duration> {
        if attempt >= self.config.max_attempts || elapsed >= self.config.max_total_wait {
            return None;
        }
        let remaining = self.config.max_total_wait.saturating_sub(elapsed);
        Some(self.delay_for_attempt(attempt).min(remaining))
    }
}

fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

/// Derives a deterministic 64-bit jitter seed from `(session_id, holder_pid)`.
pub fn seed_for_session(session_id: &str, holder_pid: u32) -> u64 {
    let mut hasher = Sha256::new();
    hasher.update(session_id.as_bytes());
    hasher.update(b":");
    hasher.update(holder_pid.to_le_bytes());
    let digest = hasher.finalize();
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&digest[..8]);
    u64::from_le_bytes(bytes)
}

/// Explicit owner consent scope granted to startup automation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnerConsentScope {
    pub allow_folder_trust: bool,
    pub allow_startup_authorization: bool,
    pub allow_onboarding_defaults: bool,
    pub prefer_remember_folder: bool,
    pub granted_workspace: Option<PathBuf>,
}

impl OwnerConsentScope {
    /// Strictest scope: no startup prompts are auto-approved.
    pub fn strict() -> Self {
        Self {
            allow_folder_trust: false,
            allow_startup_authorization: false,
            allow_onboarding_defaults: false,
            prefer_remember_folder: false,
            granted_workspace: None,
        }
    }

    /// Grants single-session startup trust, authorization, and onboarding
    /// defaults strictly scoped to `path`.
    pub fn local_workspace(path: PathBuf) -> Self {
        Self {
            allow_folder_trust: true,
            allow_startup_authorization: true,
            allow_onboarding_defaults: true,
            prefer_remember_folder: false,
            granted_workspace: Some(path),
        }
    }
}

/// Classification of a TUI prompt observed during session startup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupPromptClass {
    FolderTrust,
    StartupAuthorization,
    Onboarding,
    YesNoConfirmation,
    ToolPermission,
    McpPermission,
    LoginRequired,
    DestructiveOrExternal,
    Ambiguous,
}

/// How the terminal prompt accepts key input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyMappingStyle {
    NumberedMenu,
    CursorList,
    InlineYesNo { default_yes: bool, full_word: bool },
    HotkeyHint,
    Unknown,
}

/// Deterministic key input sent to the terminal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyInput {
    Enter,
    Escape,
    Up,
    Down,
    Char(char),
    Text(String),
}

impl KeyInput {
    /// Encodes the key input into terminal wire bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        match self {
            Self::Enter => vec![b'\r'],
            Self::Escape => vec![0x1b],
            Self::Up => b"\x1b[A".to_vec(),
            Self::Down => b"\x1b[B".to_vec(),
            Self::Char(c) => c.to_string().into_bytes(),
            Self::Text(text) => text.as_bytes().to_vec(),
        }
    }
}

/// Semantic meaning of an individual menu choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChoiceSemantic {
    ApproveOnce,
    ApproveAndRemember,
    DenyOrCancel,
    SafeDefault,
    Unknown,
}

/// A parsed selectable choice on a startup TUI screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedChoice {
    pub index: usize,
    pub label: String,
    pub focused: bool,
    pub hotkey: Option<char>,
    pub semantic: ChoiceSemantic,
}

/// Structured representation of an active startup prompt parsed from a screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedStartupPrompt {
    pub class: StartupPromptClass,
    pub key_mapping: KeyMappingStyle,
    pub choices: Vec<ParsedChoice>,
    pub risk_signals: Vec<String>,
    pub detected_workspace: Option<String>,
    pub summary_line: String,
}

const RISK_SIGNAL_TOKENS: &[&str] = &[
    "rm -rf",
    "force",
    "sudo",
    "drop ",
    "delete",
    "push",
    "publish",
    "send email",
    "external",
    "network",
    "http://",
    "https://",
    "token",
    "keychain",
    "credentials",
    "all workspaces",
    "always allow",
    "full disk",
];

const BENIGN_STARTUP_AUTH_PHRASES: &[&str] = &[
    "do you authorize",
    "authorize me",
    "authorize antigravity",
    "authorize agy",
    "authorize this workspace",
    "authorize this session",
];

/// Parses an active startup prompt from `screen`, or returns `None` for normal
/// ready/idle/streaming screens.
pub fn parse_startup_prompt(screen: &str) -> Option<ParsedStartupPrompt> {
    let plain_text = plain(screen);
    if plain_text.is_empty() {
        return None;
    }
    let lower = plain_text.to_ascii_lowercase();

    let base_prompt = screen_question::prompt_on(screen);
    let inline_yes_no = detect_inline_yes_no(&lower);
    let hotkey_choices = detect_hotkey_choices(screen);

    if base_prompt.is_none() && inline_yes_no.is_none() && hotkey_choices.is_empty() {
        return None;
    }

    let numbered_choices = extract_numbered_parsed_choices(screen);
    let cursor_choices = if numbered_choices.len() >= 2 {
        Vec::new()
    } else {
        extract_cursor_parsed_choices(screen)
    };

    let (key_mapping, mut choices) = if numbered_choices.len() >= 2 {
        (KeyMappingStyle::NumberedMenu, numbered_choices)
    } else if cursor_choices.len() >= 2 {
        (KeyMappingStyle::CursorList, cursor_choices)
    } else if let Some((default_yes, full_word)) = inline_yes_no {
        (
            KeyMappingStyle::InlineYesNo {
                default_yes,
                full_word,
            },
            vec![
                ParsedChoice {
                    index: 0,
                    label: if full_word {
                        "yes".to_owned()
                    } else {
                        "y".to_owned()
                    },
                    focused: default_yes,
                    hotkey: Some('y'),
                    semantic: ChoiceSemantic::ApproveOnce,
                },
                ParsedChoice {
                    index: 1,
                    label: if full_word {
                        "no".to_owned()
                    } else {
                        "n".to_owned()
                    },
                    focused: !default_yes,
                    hotkey: Some('n'),
                    semantic: ChoiceSemantic::DenyOrCancel,
                },
            ],
        )
    } else if !hotkey_choices.is_empty() {
        (KeyMappingStyle::HotkeyHint, hotkey_choices)
    } else {
        (KeyMappingStyle::Unknown, Vec::new())
    };

    let risk_signals = detect_risk_signals(&lower);
    let detected_workspace = detect_workspace_path(screen);

    let mut class = match base_prompt.as_ref().map(|p| p.kind) {
        Some(PromptKind::FolderTrust) => StartupPromptClass::FolderTrust,
        Some(PromptKind::Authorization) => StartupPromptClass::StartupAuthorization,
        Some(PromptKind::Onboarding) => StartupPromptClass::Onboarding,
        Some(PromptKind::Permission) | Some(PromptKind::Plan) => StartupPromptClass::ToolPermission,
        Some(PromptKind::Mcp) => StartupPromptClass::McpPermission,
        Some(PromptKind::Login) => StartupPromptClass::LoginRequired,
        Some(PromptKind::Question) => {
            if inline_yes_no.is_some() || !choices.is_empty() {
                if BENIGN_STARTUP_AUTH_PHRASES
                    .iter()
                    .any(|phrase| lower.contains(phrase))
                {
                    StartupPromptClass::StartupAuthorization
                } else if inline_yes_no.is_some() {
                    StartupPromptClass::YesNoConfirmation
                } else {
                    StartupPromptClass::Ambiguous
                }
            } else {
                StartupPromptClass::Ambiguous
            }
        }
        None => {
            if BENIGN_STARTUP_AUTH_PHRASES
                .iter()
                .any(|phrase| lower.contains(phrase))
            {
                StartupPromptClass::StartupAuthorization
            } else if inline_yes_no.is_some() {
                StartupPromptClass::YesNoConfirmation
            } else {
                StartupPromptClass::Ambiguous
            }
        }
    };

    if class == StartupPromptClass::Onboarding {
        for choice in &mut choices {
            let label_lower = choice.label.to_ascii_lowercase();
            if choice.focused || label_lower.contains("auto") || label_lower.contains("default") {
                choice.semantic = ChoiceSemantic::SafeDefault;
            }
        }
    }

    if !risk_signals.is_empty() {
        class = StartupPromptClass::DestructiveOrExternal;
    }

    let summary_line = extract_summary_line(screen, &plain_text);

    Some(ParsedStartupPrompt {
        class,
        key_mapping,
        choices,
        risk_signals,
        detected_workspace,
        summary_line,
    })
}

/// Returns `true` when `screen` shows a non-empty, ready terminal screen with
/// no active startup prompt and no loading-only blank state.
pub fn is_ready_screen(screen: &str) -> bool {
    let plain_text = plain(screen);
    if plain_text.is_empty() {
        return false;
    }
    if parse_startup_prompt(screen).is_some() {
        return false;
    }
    let lower = plain_text.to_ascii_lowercase();
    let loading_only = matches!(
        lower.trim(),
        "loading..."
            | "loading"
            | "starting..."
            | "starting"
            | "initializing..."
            | "initializing"
            | "booting..."
            | "booting"
            | "..."
    );
    !loading_only
}

/// Reversibility classification for an approval review boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reversibility {
    Reversible,
    PartiallyReversible,
    Irreversible,
    Unknown,
}

/// Why startup automation paused for smart-agent or human review.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PauseReason {
    OutOfGrantedScope,
    DestructiveOrExternalSignal,
    AmbiguousPrompt,
    NoSafeKeyMapping,
    LoginOrExternalAuth,
    TimeoutExceeded,
    StuckAfterKeyPress,
    ScreenReadError,
    KeySendError,
}

/// Structured summary surfaced when startup automation pauses for review.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalReviewSummary {
    pub action: String,
    pub target: String,
    pub scope: String,
    pub duration: String,
    pub reversibility: Reversibility,
    pub changed_risk_factors: Vec<String>,
    pub reason: PauseReason,
    pub requires_smart_agent_review: bool,
}

/// Deterministic policy decision for a parsed startup prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartupDecision {
    PressKeys {
        keys: Vec<KeyInput>,
        chosen_label: String,
        rationale: String,
    },
    PauseForReview(ApprovalReviewSummary),
}

/// Evaluates `prompt` against `scope` and produces a deterministic key sequence
/// or a fail-safe pause summary for smart-agent/human review.
pub fn decide_startup_action(
    prompt: &ParsedStartupPrompt,
    scope: &OwnerConsentScope,
) -> StartupDecision {
    if !prompt.risk_signals.is_empty() || prompt.class == StartupPromptClass::DestructiveOrExternal
    {
        return StartupDecision::PauseForReview(ApprovalReviewSummary {
            action: prompt.summary_line.clone(),
            target: prompt
                .detected_workspace
                .clone()
                .unwrap_or_else(|| "unspecified target".to_owned()),
            scope: "destructive or external operation".to_owned(),
            duration: "persistent or external".to_owned(),
            reversibility: Reversibility::Irreversible,
            changed_risk_factors: if prompt.risk_signals.is_empty() {
                vec!["destructive_or_external_class".to_owned()]
            } else {
                prompt.risk_signals.clone()
            },
            reason: PauseReason::DestructiveOrExternalSignal,
            requires_smart_agent_review: true,
        });
    }

    match prompt.class {
        StartupPromptClass::LoginRequired => {
            StartupDecision::PauseForReview(ApprovalReviewSummary {
                action: prompt.summary_line.clone(),
                target: "account authentication".to_owned(),
                scope: "external login / credentials".to_owned(),
                duration: "account session".to_owned(),
                reversibility: Reversibility::PartiallyReversible,
                changed_risk_factors: vec!["external_authentication_required".to_owned()],
                reason: PauseReason::LoginOrExternalAuth,
                requires_smart_agent_review: true,
            })
        }
        StartupPromptClass::ToolPermission | StartupPromptClass::McpPermission => {
            StartupDecision::PauseForReview(ApprovalReviewSummary {
                action: prompt.summary_line.clone(),
                target: "tool or MCP execution".to_owned(),
                scope: "runtime tool permission".to_owned(),
                duration: "single or persistent tool approval".to_owned(),
                reversibility: Reversibility::Unknown,
                changed_risk_factors: vec![format!("prompt_class:{:?}", prompt.class)],
                reason: PauseReason::OutOfGrantedScope,
                requires_smart_agent_review: true,
            })
        }
        StartupPromptClass::Ambiguous | StartupPromptClass::DestructiveOrExternal => {
            StartupDecision::PauseForReview(ApprovalReviewSummary {
                action: prompt.summary_line.clone(),
                target: prompt
                    .detected_workspace
                    .clone()
                    .unwrap_or_else(|| "unknown".to_owned()),
                scope: "unclassified startup prompt".to_owned(),
                duration: "unknown".to_owned(),
                reversibility: Reversibility::Unknown,
                changed_risk_factors: vec!["ambiguous_prompt".to_owned()],
                reason: PauseReason::AmbiguousPrompt,
                requires_smart_agent_review: true,
            })
        }
        StartupPromptClass::FolderTrust => {
            if !scope.allow_folder_trust {
                return StartupDecision::PauseForReview(ApprovalReviewSummary {
                    action: "approve folder trust".to_owned(),
                    target: prompt
                        .detected_workspace
                        .clone()
                        .unwrap_or_else(|| "current workspace".to_owned()),
                    scope: "folder trust not granted by owner consent".to_owned(),
                    duration: "current session".to_owned(),
                    reversibility: Reversibility::Reversible,
                    changed_risk_factors: vec!["allow_folder_trust=false".to_owned()],
                    reason: PauseReason::OutOfGrantedScope,
                    requires_smart_agent_review: true,
                });
            }
            if let (Some(granted), Some(detected)) =
                (&scope.granted_workspace, &prompt.detected_workspace)
            {
                if !workspace_matches(granted, detected) {
                    return StartupDecision::PauseForReview(ApprovalReviewSummary {
                        action: "approve folder trust".to_owned(),
                        target: detected.clone(),
                        scope: format!("granted workspace is {}", granted.display()),
                        duration: "current session".to_owned(),
                        reversibility: Reversibility::Reversible,
                        changed_risk_factors: vec![format!(
                            "workspace_mismatch: detected={}, granted={}",
                            detected,
                            granted.display()
                        )],
                        reason: PauseReason::OutOfGrantedScope,
                        requires_smart_agent_review: true,
                    });
                }
            }
            let preferred = if scope.prefer_remember_folder {
                prompt
                    .choices
                    .iter()
                    .find(|c| c.semantic == ChoiceSemantic::ApproveAndRemember)
                    .or_else(|| {
                        prompt
                            .choices
                            .iter()
                            .find(|c| c.semantic == ChoiceSemantic::ApproveOnce)
                    })
            } else {
                prompt
                    .choices
                    .iter()
                    .find(|c| c.semantic == ChoiceSemantic::ApproveOnce)
            };
            let Some(target_choice) = preferred else {
                return pause_no_safe_key_mapping(prompt, "no safe folder trust approval choice");
            };
            select_keys_for_choice(
                prompt,
                target_choice,
                "approved folder trust within granted workspace scope",
            )
        }
        StartupPromptClass::StartupAuthorization => {
            if !scope.allow_startup_authorization {
                return StartupDecision::PauseForReview(ApprovalReviewSummary {
                    action: prompt.summary_line.clone(),
                    target: prompt
                        .detected_workspace
                        .clone()
                        .unwrap_or_else(|| "startup session".to_owned()),
                    scope: "startup authorization not granted by owner consent".to_owned(),
                    duration: "current session".to_owned(),
                    reversibility: Reversibility::Reversible,
                    changed_risk_factors: vec!["allow_startup_authorization=false".to_owned()],
                    reason: PauseReason::OutOfGrantedScope,
                    requires_smart_agent_review: true,
                });
            }
            if let (Some(granted), Some(detected)) =
                (&scope.granted_workspace, &prompt.detected_workspace)
            {
                if !workspace_matches(granted, detected) {
                    return StartupDecision::PauseForReview(ApprovalReviewSummary {
                        action: prompt.summary_line.clone(),
                        target: detected.clone(),
                        scope: format!("granted workspace is {}", granted.display()),
                        duration: "current session".to_owned(),
                        reversibility: Reversibility::Reversible,
                        changed_risk_factors: vec![format!(
                            "workspace_mismatch: detected={}, granted={}",
                            detected,
                            granted.display()
                        )],
                        reason: PauseReason::OutOfGrantedScope,
                        requires_smart_agent_review: true,
                    });
                }
            }
            let Some(target_choice) = prompt
                .choices
                .iter()
                .find(|c| c.semantic == ChoiceSemantic::ApproveOnce)
            else {
                return pause_no_safe_key_mapping(
                    prompt,
                    "no safe single-session authorization choice",
                );
            };
            select_keys_for_choice(
                prompt,
                target_choice,
                "approved startup authorization within granted owner consent",
            )
        }
        StartupPromptClass::Onboarding => {
            if !scope.allow_onboarding_defaults {
                return StartupDecision::PauseForReview(ApprovalReviewSummary {
                    action: prompt.summary_line.clone(),
                    target: "CLI onboarding setup".to_owned(),
                    scope: "onboarding defaults not granted by owner consent".to_owned(),
                    duration: "CLI profile preference".to_owned(),
                    reversibility: Reversibility::Reversible,
                    changed_risk_factors: vec!["allow_onboarding_defaults=false".to_owned()],
                    reason: PauseReason::OutOfGrantedScope,
                    requires_smart_agent_review: true,
                });
            }
            if let Some(focused_safe) = prompt
                .choices
                .iter()
                .find(|c| c.focused && c.semantic == ChoiceSemantic::SafeDefault)
            {
                return StartupDecision::PressKeys {
                    keys: vec![KeyInput::Enter],
                    chosen_label: focused_safe.label.clone(),
                    rationale: "accepted focused safe onboarding default".to_owned(),
                };
            }
            pause_no_safe_key_mapping(prompt, "no focused safe onboarding default")
        }
        StartupPromptClass::YesNoConfirmation => {
            let lower_summary = prompt.summary_line.to_ascii_lowercase();
            let benign_auth = BENIGN_STARTUP_AUTH_PHRASES
                .iter()
                .any(|phrase| lower_summary.contains(phrase));
            if scope.allow_startup_authorization && benign_auth {
                if let Some(target_choice) = prompt
                    .choices
                    .iter()
                    .find(|c| c.semantic == ChoiceSemantic::ApproveOnce)
                {
                    return select_keys_for_choice(
                        prompt,
                        target_choice,
                        "approved benign startup confirmation",
                    );
                }
            }
            StartupDecision::PauseForReview(ApprovalReviewSummary {
                action: prompt.summary_line.clone(),
                target: prompt
                    .detected_workspace
                    .clone()
                    .unwrap_or_else(|| "unknown confirmation".to_owned()),
                scope: "generic yes/no confirmation outside explicit benign startup auth"
                    .to_owned(),
                duration: "unknown".to_owned(),
                reversibility: Reversibility::Unknown,
                changed_risk_factors: vec!["ambiguous_yes_no_confirmation".to_owned()],
                reason: PauseReason::AmbiguousPrompt,
                requires_smart_agent_review: true,
            })
        }
    }
}

fn pause_no_safe_key_mapping(prompt: &ParsedStartupPrompt, detail: &str) -> StartupDecision {
    StartupDecision::PauseForReview(ApprovalReviewSummary {
        action: prompt.summary_line.clone(),
        target: prompt
            .detected_workspace
            .clone()
            .unwrap_or_else(|| "startup prompt".to_owned()),
        scope: detail.to_owned(),
        duration: "current session".to_owned(),
        reversibility: Reversibility::Reversible,
        changed_risk_factors: vec![detail.to_owned()],
        reason: PauseReason::NoSafeKeyMapping,
        requires_smart_agent_review: true,
    })
}

fn select_keys_for_choice(
    prompt: &ParsedStartupPrompt,
    target: &ParsedChoice,
    rationale: &str,
) -> StartupDecision {
    let focused_idx = prompt
        .choices
        .iter()
        .find(|c| c.focused)
        .map(|c| c.index);

    match prompt.key_mapping {
        KeyMappingStyle::CursorList => {
            let Some(cur) = focused_idx else {
                return pause_no_safe_key_mapping(
                    prompt,
                    "cursor list has no identifiable focused item",
                );
            };
            let mut keys = navigation_keys(cur, target.index);
            keys.push(KeyInput::Enter);
            StartupDecision::PressKeys {
                keys,
                chosen_label: target.label.clone(),
                rationale: rationale.to_owned(),
            }
        }
        KeyMappingStyle::NumberedMenu => {
            if focused_idx == Some(target.index) {
                StartupDecision::PressKeys {
                    keys: vec![KeyInput::Enter],
                    chosen_label: target.label.clone(),
                    rationale: rationale.to_owned(),
                }
            } else if let Some(cur) = focused_idx {
                let mut keys = navigation_keys(cur, target.index);
                keys.push(KeyInput::Enter);
                StartupDecision::PressKeys {
                    keys,
                    chosen_label: target.label.clone(),
                    rationale: rationale.to_owned(),
                }
            } else if target.index < 9 {
                let digit = char::from(b'1' + (target.index as u8));
                StartupDecision::PressKeys {
                    keys: vec![KeyInput::Char(digit), KeyInput::Enter],
                    chosen_label: target.label.clone(),
                    rationale: rationale.to_owned(),
                }
            } else {
                pause_no_safe_key_mapping(prompt, "numbered menu target exceeds single-digit key")
            }
        }
        KeyMappingStyle::InlineYesNo { full_word, .. } => {
            let keys = if full_word {
                vec![KeyInput::Text("yes".to_owned()), KeyInput::Enter]
            } else {
                vec![KeyInput::Char('y'), KeyInput::Enter]
            };
            StartupDecision::PressKeys {
                keys,
                chosen_label: target.label.clone(),
                rationale: rationale.to_owned(),
            }
        }
        KeyMappingStyle::HotkeyHint => {
            if let Some(hotkey) = target.hotkey {
                StartupDecision::PressKeys {
                    keys: vec![KeyInput::Char(hotkey), KeyInput::Enter],
                    chosen_label: target.label.clone(),
                    rationale: rationale.to_owned(),
                }
            } else if target.focused {
                StartupDecision::PressKeys {
                    keys: vec![KeyInput::Enter],
                    chosen_label: target.label.clone(),
                    rationale: rationale.to_owned(),
                }
            } else {
                pause_no_safe_key_mapping(prompt, "hotkey prompt has no safe key for target choice")
            }
        }
        KeyMappingStyle::Unknown => {
            pause_no_safe_key_mapping(prompt, "unknown key mapping style for startup prompt")
        }
    }
}

fn navigation_keys(from: usize, to: usize) -> Vec<KeyInput> {
    if to > from {
        vec![KeyInput::Down; to - from]
    } else if from > to {
        vec![KeyInput::Up; from - to]
    } else {
        Vec::new()
    }
}

fn workspace_matches(granted: &Path, detected: &str) -> bool {
    let norm_granted = granted.to_string_lossy().trim_end_matches('/').to_owned();
    let norm_detected = detected.trim().trim_end_matches('/').to_owned();
    norm_granted == norm_detected
}

/// Outcome recorded for each attempt in [`StartupAuditTrail`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditOutcome {
    NoPromptYet,
    Ready,
    KeysSent,
    PausedForReview,
    Error,
}

/// A single auditable observation/action during startup authorization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditEntry {
    pub attempt: usize,
    pub elapsed_ms: u64,
    pub scheduled_delay_ms: u64,
    pub screen_hash: String,
    pub screen_excerpt: String,
    pub classification: Option<StartupPromptClass>,
    pub key_mapping: Option<KeyMappingStyle>,
    pub keys_pressed: Vec<KeyInput>,
    pub outcome: AuditOutcome,
    pub rationale: String,
}

/// Complete structured audit trail for a startup authorization run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupAuditTrail {
    pub session_id: String,
    pub agent_kind: String,
    pub seed: u64,
    pub entries: Vec<AuditEntry>,
}

/// Terminal port used by [`StartupAuthRunner`] to inspect screens and send keys.
#[tonic::async_trait]
pub trait StartupTerminalPort: Send + Sync {
    async fn capture_screen(&self) -> Result<Option<(u64, String)>, StartupAuthError>;
    async fn send_keys(&self, keys: &[KeyInput]) -> Result<(), StartupAuthError>;
}

/// Terminal outcome of a startup authorization run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartupStatus {
    Ready,
    PausedForReview(ApprovalReviewSummary),
}

/// Final report returned by [`StartupAuthRunner::run`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupAuthReport {
    pub status: StartupStatus,
    pub audit: StartupAuditTrail,
    pub total_keys_sent: usize,
}

/// Deterministic, bounded async runner for TUI startup authorization.
#[derive(Debug, Clone)]
pub struct StartupAuthRunner {
    session_id: String,
    agent_kind: String,
    schedule: StartupSchedule,
}

impl StartupAuthRunner {
    /// Creates a new runner with validated startup schedule configuration.
    pub fn new(
        session_id: impl Into<String>,
        agent_kind: impl Into<String>,
        config: StartupConfig,
    ) -> Result<Self, StartupAuthError> {
        let schedule = StartupSchedule::new(config)?;
        Ok(Self {
            session_id: session_id.into(),
            agent_kind: agent_kind.into(),
            schedule,
        })
    }

    /// Runs the bounded startup loop against `terminal` under `scope`.
    pub async fn run(
        &self,
        terminal: &dyn StartupTerminalPort,
        scope: &OwnerConsentScope,
    ) -> StartupAuthReport {
        let start = tokio::time::Instant::now();
        let initial_wait = self.schedule.config().initial_wait;
        if !initial_wait.is_zero() {
            tokio::time::sleep(initial_wait).await;
        }

        let mut entries = Vec::new();
        let mut total_keys_sent = 0usize;
        let mut last_keys_screen_hash: Option<String> = None;

        for attempt in 0..self.schedule.config().max_attempts {
            let elapsed = start.elapsed();
            let elapsed_ms = elapsed.as_millis().min(u128::from(u64::MAX)) as u64;

            let capture = match terminal.capture_screen().await {
                Ok(capture) => capture,
                Err(err) => {
                    let summary = ApprovalReviewSummary {
                        action: "read startup terminal screen".to_owned(),
                        target: self.session_id.clone(),
                        scope: "terminal screen capture".to_owned(),
                        duration: "startup".to_owned(),
                        reversibility: Reversibility::Reversible,
                        changed_risk_factors: vec![err.to_string()],
                        reason: PauseReason::ScreenReadError,
                        requires_smart_agent_review: true,
                    };
                    entries.push(AuditEntry {
                        attempt,
                        elapsed_ms,
                        scheduled_delay_ms: 0,
                        screen_hash: String::new(),
                        screen_excerpt: String::new(),
                        classification: None,
                        key_mapping: None,
                        keys_pressed: Vec::new(),
                        outcome: AuditOutcome::Error,
                        rationale: err.to_string(),
                    });
                    return StartupAuthReport {
                        status: StartupStatus::PausedForReview(summary),
                        audit: StartupAuditTrail {
                            session_id: self.session_id.clone(),
                            agent_kind: self.agent_kind.clone(),
                            seed: self.schedule.config().seed,
                            entries,
                        },
                        total_keys_sent,
                    };
                }
            };

            if let Some((_end, screen)) = capture {
                let screen_hash = sha256_hex(&screen);
                let screen_excerpt = excerpt(&screen);

                if let Some(prompt) = parse_startup_prompt(&screen) {
                    if last_keys_screen_hash.as_deref() == Some(screen_hash.as_str()) {
                        let summary = ApprovalReviewSummary {
                            action: prompt.summary_line.clone(),
                            target: prompt
                                .detected_workspace
                                .clone()
                                .unwrap_or_else(|| self.session_id.clone()),
                            scope: "startup prompt did not advance after sending keys".to_owned(),
                            duration: "startup".to_owned(),
                            reversibility: Reversibility::Reversible,
                            changed_risk_factors: vec!["unchanged_screen_hash_after_keys".to_owned()],
                            reason: PauseReason::StuckAfterKeyPress,
                            requires_smart_agent_review: true,
                        };
                        entries.push(AuditEntry {
                            attempt,
                            elapsed_ms,
                            scheduled_delay_ms: 0,
                            screen_hash,
                            screen_excerpt,
                            classification: Some(prompt.class),
                            key_mapping: Some(prompt.key_mapping),
                            keys_pressed: Vec::new(),
                            outcome: AuditOutcome::PausedForReview,
                            rationale: "screen unchanged after previous key press".to_owned(),
                        });
                        return StartupAuthReport {
                            status: StartupStatus::PausedForReview(summary),
                            audit: StartupAuditTrail {
                                session_id: self.session_id.clone(),
                                agent_kind: self.agent_kind.clone(),
                                seed: self.schedule.config().seed,
                                entries,
                            },
                            total_keys_sent,
                        };
                    }

                    match decide_startup_action(&prompt, scope) {
                        StartupDecision::PressKeys {
                            keys,
                            chosen_label,
                            rationale,
                        } => {
                            if let Err(err) = terminal.send_keys(&keys).await {
                                let summary = ApprovalReviewSummary {
                                    action: format!("send keys for '{chosen_label}'"),
                                    target: self.session_id.clone(),
                                    scope: "terminal key dispatch".to_owned(),
                                    duration: "startup".to_owned(),
                                    reversibility: Reversibility::Reversible,
                                    changed_risk_factors: vec![err.to_string()],
                                    reason: PauseReason::KeySendError,
                                    requires_smart_agent_review: true,
                                };
                                entries.push(AuditEntry {
                                    attempt,
                                    elapsed_ms,
                                    scheduled_delay_ms: 0,
                                    screen_hash,
                                    screen_excerpt,
                                    classification: Some(prompt.class),
                                    key_mapping: Some(prompt.key_mapping),
                                    keys_pressed: Vec::new(),
                                    outcome: AuditOutcome::Error,
                                    rationale: err.to_string(),
                                });
                                return StartupAuthReport {
                                    status: StartupStatus::PausedForReview(summary),
                                    audit: StartupAuditTrail {
                                        session_id: self.session_id.clone(),
                                        agent_kind: self.agent_kind.clone(),
                                        seed: self.schedule.config().seed,
                                        entries,
                                    },
                                    total_keys_sent,
                                };
                            }
                            total_keys_sent += keys.len();
                            last_keys_screen_hash = Some(screen_hash.clone());
                            let next_delay = self.schedule.bounded_delay(attempt, start.elapsed());
                            let scheduled_delay_ms = next_delay
                                .map(|d| d.as_millis().min(u128::from(u64::MAX)) as u64)
                                .unwrap_or(0);
                            entries.push(AuditEntry {
                                attempt,
                                elapsed_ms,
                                scheduled_delay_ms,
                                screen_hash,
                                screen_excerpt,
                                classification: Some(prompt.class),
                                key_mapping: Some(prompt.key_mapping),
                                keys_pressed: keys,
                                outcome: AuditOutcome::KeysSent,
                                rationale: format!("{rationale} (choice: {chosen_label})"),
                            });
                            if let Some(delay) = next_delay {
                                if !delay.is_zero() {
                                    tokio::time::sleep(delay).await;
                                }
                                continue;
                            }
                            break;
                        }
                        StartupDecision::PauseForReview(summary) => {
                            entries.push(AuditEntry {
                                attempt,
                                elapsed_ms,
                                scheduled_delay_ms: 0,
                                screen_hash,
                                screen_excerpt,
                                classification: Some(prompt.class),
                                key_mapping: Some(prompt.key_mapping),
                                keys_pressed: Vec::new(),
                                outcome: AuditOutcome::PausedForReview,
                                rationale: format!("paused for review: {:?}", summary.reason),
                            });
                            return StartupAuthReport {
                                status: StartupStatus::PausedForReview(summary),
                                audit: StartupAuditTrail {
                                    session_id: self.session_id.clone(),
                                    agent_kind: self.agent_kind.clone(),
                                    seed: self.schedule.config().seed,
                                    entries,
                                },
                                total_keys_sent,
                            };
                        }
                    }
                } else if is_ready_screen(&screen) {
                    entries.push(AuditEntry {
                        attempt,
                        elapsed_ms,
                        scheduled_delay_ms: 0,
                        screen_hash,
                        screen_excerpt,
                        classification: None,
                        key_mapping: None,
                        keys_pressed: Vec::new(),
                        outcome: AuditOutcome::Ready,
                        rationale: "terminal reached ready state".to_owned(),
                    });
                    return StartupAuthReport {
                        status: StartupStatus::Ready,
                        audit: StartupAuditTrail {
                            session_id: self.session_id.clone(),
                            agent_kind: self.agent_kind.clone(),
                            seed: self.schedule.config().seed,
                            entries,
                        },
                        total_keys_sent,
                    };
                } else {
                    let next_delay = self.schedule.bounded_delay(attempt, start.elapsed());
                    let scheduled_delay_ms = next_delay
                        .map(|d| d.as_millis().min(u128::from(u64::MAX)) as u64)
                        .unwrap_or(0);
                    entries.push(AuditEntry {
                        attempt,
                        elapsed_ms,
                        scheduled_delay_ms,
                        screen_hash,
                        screen_excerpt,
                        classification: None,
                        key_mapping: None,
                        keys_pressed: Vec::new(),
                        outcome: AuditOutcome::NoPromptYet,
                        rationale: "screen still loading".to_owned(),
                    });
                    if let Some(delay) = next_delay {
                        if !delay.is_zero() {
                            tokio::time::sleep(delay).await;
                        }
                        continue;
                    }
                    break;
                }
            } else {
                let next_delay = self.schedule.bounded_delay(attempt, start.elapsed());
                let scheduled_delay_ms = next_delay
                    .map(|d| d.as_millis().min(u128::from(u64::MAX)) as u64)
                    .unwrap_or(0);
                entries.push(AuditEntry {
                    attempt,
                    elapsed_ms,
                    scheduled_delay_ms,
                    screen_hash: String::new(),
                    screen_excerpt: String::new(),
                    classification: None,
                    key_mapping: None,
                    keys_pressed: Vec::new(),
                    outcome: AuditOutcome::NoPromptYet,
                    rationale: "no screen output yet".to_owned(),
                });
                if let Some(delay) = next_delay {
                    if !delay.is_zero() {
                        tokio::time::sleep(delay).await;
                    }
                    continue;
                }
                break;
            }
        }

        let elapsed_ms = start.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
        let summary = ApprovalReviewSummary {
            action: "wait for startup readiness".to_owned(),
            target: self.session_id.clone(),
            scope: "bounded startup window exhausted".to_owned(),
            duration: format!("{elapsed_ms}ms"),
            reversibility: Reversibility::Reversible,
            changed_risk_factors: vec!["startup_timeout_exceeded".to_owned()],
            reason: PauseReason::TimeoutExceeded,
            requires_smart_agent_review: true,
        };
        entries.push(AuditEntry {
            attempt: entries.len(),
            elapsed_ms,
            scheduled_delay_ms: 0,
            screen_hash: String::new(),
            screen_excerpt: String::new(),
            classification: None,
            key_mapping: None,
            keys_pressed: Vec::new(),
            outcome: AuditOutcome::PausedForReview,
            rationale: "startup budget exhausted without reaching ready state".to_owned(),
        });

        StartupAuthReport {
            status: StartupStatus::PausedForReview(summary),
            audit: StartupAuditTrail {
                session_id: self.session_id.clone(),
                agent_kind: self.agent_kind.clone(),
                seed: self.schedule.config().seed,
                entries,
            },
            total_keys_sent,
        }
    }
}

fn sha256_hex(text: &str) -> String {
    let digest = Sha256::digest(text.as_bytes());
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn excerpt(screen: &str) -> String {
    let collapsed = plain(screen);
    if collapsed.chars().count() <= 200 {
        collapsed
    } else {
        collapsed.chars().take(200).collect()
    }
}

fn plain(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut space = false;
    for c in text.chars() {
        if c.is_whitespace() || ('\u{2500}'..='\u{257f}').contains(&c) {
            space = true;
            continue;
        }
        if space && !out.is_empty() {
            out.push(' ');
        }
        space = false;
        out.push(c);
    }
    out
}

fn unbox_line(line: &str) -> &str {
    line.trim_matches(|c: char| c.is_whitespace() || ('\u{2500}'..='\u{257f}').contains(&c))
}

fn collapse_spaces(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn strip_focus_marker(line: &str) -> (bool, &str) {
    for marker in ["❯ ", "› ", "> "] {
        if let Some(rest) = line.strip_prefix(marker) {
            return (true, rest.trim_start());
        }
    }
    (false, line)
}

fn detect_risk_signals(lower: &str) -> Vec<String> {
    RISK_SIGNAL_TOKENS
        .iter()
        .filter(|token| lower.contains(**token))
        .map(|token| (*token).to_owned())
        .collect()
}

fn detect_workspace_path(screen: &str) -> Option<String> {
    for raw in screen.lines() {
        let line = unbox_line(raw);
        if line.is_empty() {
            continue;
        }
        if (line.starts_with('/') || line.starts_with("~/"))
            && !line.contains(' ')
            && line.len() > 1
        {
            return Some(line.to_owned());
        }
    }
    None
}

fn detect_inline_yes_no(lower: &str) -> Option<(bool, bool)> {
    let trimmed = lower.trim_end();
    if trimmed.ends_with("[y/n]") || trimmed.ends_with("[y/n]?") {
        // Distinguish [Y/n] vs [y/N] by checking original case? Here `lower` is
        // lowercased, so default_yes is false unless caller checks case; let's
        // handle case-insensitive here and keep default_yes = false safely.
        return Some((false, false));
    }
    if trimmed.ends_with("(y/n)") || trimmed.ends_with("(y/n)?") {
        return Some((false, false));
    }
    if trimmed.ends_with("(yes/no)")
        || trimmed.ends_with("[yes/no]")
        || trimmed.ends_with("(yes/no)?")
        || trimmed.ends_with("[yes/no]?")
    {
        return Some((false, true));
    }
    None
}

fn detect_hotkey_choices(screen: &str) -> Vec<ParsedChoice> {
    let lower = plain(screen).to_ascii_lowercase();
    let has_y = lower.contains("(y)") || lower.contains("[y]");
    let has_n = lower.contains("(n)") || lower.contains("[n]");
    if has_y && has_n {
        vec![
            ParsedChoice {
                index: 0,
                label: "Yes (y)".to_owned(),
                focused: false,
                hotkey: Some('y'),
                semantic: ChoiceSemantic::ApproveOnce,
            },
            ParsedChoice {
                index: 1,
                label: "No (n)".to_owned(),
                focused: false,
                hotkey: Some('n'),
                semantic: ChoiceSemantic::DenyOrCancel,
            },
        ]
    } else {
        Vec::new()
    }
}

fn classify_choice_semantic(label: &str) -> ChoiceSemantic {
    let lower = label.to_ascii_lowercase();
    if lower.contains("remember")
        || lower.contains("future sessions")
        || lower.contains("don't ask again")
        || lower.contains("always")
    {
        return ChoiceSemantic::ApproveAndRemember;
    }
    if lower == "no"
        || lower.starts_with("no,")
        || lower.starts_with("no ")
        || lower.starts_with("no(")
        || lower.contains("exit")
        || lower.contains("cancel")
        || lower.contains("deny")
        || lower.contains("decline")
        || lower.contains("back to")
        || lower.contains("not now")
    {
        return ChoiceSemantic::DenyOrCancel;
    }
    if lower == "yes"
        || lower.starts_with("yes,")
        || lower.starts_with("yes ")
        || lower.starts_with("yes(")
        || lower.contains("trust this folder")
        || lower.contains("trust and continue")
        || lower.contains("authorize")
        || lower.contains("allow once")
        || lower.contains("allow")
        || lower.contains("proceed")
        || lower.contains("confirm")
    {
        return ChoiceSemantic::ApproveOnce;
    }
    ChoiceSemantic::Unknown
}

fn extract_hotkey_from_label(label: &str) -> Option<char> {
    let bytes = label.as_bytes();
    for window in bytes.windows(3) {
        if (window[0] == b'(' && window[2] == b')') || (window[0] == b'[' && window[2] == b']') {
            let c = window[1] as char;
            if c.is_ascii_alphabetic() {
                return Some(c.to_ascii_lowercase());
            }
        }
    }
    None
}

fn parse_numbered_choice(line: &str) -> Option<(usize, bool, String)> {
    let (focused, rest) = strip_focus_marker(line);
    let (digits, after) = rest.split_once(". ")?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let num = digits.parse::<usize>().ok()?;
    let label = collapse_spaces(after);
    if label.is_empty() {
        return None;
    }
    Some((num, focused, label))
}

fn extract_numbered_parsed_choices(screen: &str) -> Vec<ParsedChoice> {
    let mut best: Vec<ParsedChoice> = Vec::new();
    let mut current: Vec<ParsedChoice> = Vec::new();
    let mut expected_num = 1usize;

    for raw in screen.lines() {
        let line = unbox_line(raw);
        if let Some((num, focused, label)) = parse_numbered_choice(line) {
            if num == 1 {
                if current.len() > best.len() {
                    best = std::mem::take(&mut current);
                } else {
                    current.clear();
                }
                let semantic = classify_choice_semantic(&label);
                let hotkey = extract_hotkey_from_label(&label);
                current.push(ParsedChoice {
                    index: 0,
                    label,
                    focused,
                    hotkey,
                    semantic,
                });
                expected_num = 2;
            } else if num == expected_num && !current.is_empty() {
                let semantic = classify_choice_semantic(&label);
                let hotkey = extract_hotkey_from_label(&label);
                current.push(ParsedChoice {
                    index: current.len(),
                    label,
                    focused,
                    hotkey,
                    semantic,
                });
                expected_num += 1;
            }
        }
    }
    if current.len() > best.len() {
        best = current;
    }
    best
}

fn is_choice_footer(lower: &str) -> bool {
    lower.starts_with("enter to ")
        || lower.starts_with("press enter")
        || lower.starts_with("↑/↓")
        || lower.starts_with("esc to ")
        || lower.starts_with("enter continue")
        || lower.starts_with("syntax theme:")
}

fn extract_cursor_parsed_choices(screen: &str) -> Vec<ParsedChoice> {
    let lines: Vec<&str> = screen.lines().collect();
    for (idx, raw) in lines.iter().enumerate() {
        let trimmed = unbox_line(raw);
        let Some(after_cursor) = trimmed.strip_prefix("❯ ") else {
            continue;
        };
        let first = collapse_spaces(after_cursor.trim_start_matches('✔').trim());
        if first.is_empty() {
            continue;
        }
        let mut choices = vec![ParsedChoice {
            index: 0,
            semantic: classify_choice_semantic(&first),
            hotkey: extract_hotkey_from_label(&first),
            label: first,
            focused: true,
        }];
        for next_raw in &lines[idx + 1..] {
            let next = unbox_line(next_raw);
            if next.is_empty()
                || next.chars().all(|c| c == '╌' || c == '─' || c == '-')
                || is_choice_footer(&next.to_ascii_lowercase())
            {
                break;
            }
            let cleaned = collapse_spaces(next.trim_start_matches('✔').trim());
            if !cleaned.is_empty() {
                let index = choices.len();
                choices.push(ParsedChoice {
                    index,
                    semantic: classify_choice_semantic(&cleaned),
                    hotkey: extract_hotkey_from_label(&cleaned),
                    label: cleaned,
                    focused: false,
                });
            }
        }
        if choices.len() >= 2 {
            return choices;
        }
    }
    Vec::new()
}

fn extract_summary_line(screen: &str, fallback_plain: &str) -> String {
    for raw in screen.lines() {
        let line = unbox_line(raw);
        if line.is_empty() {
            continue;
        }
        let lower = line.to_ascii_lowercase();
        if line.contains('?')
            || lower.contains("authorize")
            || lower.contains("trust")
            || lower.contains("choose")
            || lower.contains("allow")
            || lower.contains("sign in")
        {
            return collapse_spaces(line);
        }
    }
    fallback_plain.chars().take(120).collect()
}
