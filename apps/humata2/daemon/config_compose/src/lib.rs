//! Native per-session agent configuration composition, sync toggles,
//! secret-scrubbed FUSE views, and the 3-stage rewrite pipeline (T8).
//!
//! Invariants:
//! - Real `~/.<agent>` directories are never mutated during composition, sync,
//!   or rage-quit.
//! - Machine-local auth files are wired via native local pass-through symlinks
//!   and excluded from FUSE views (`SecretDenylist`).
//! - The protected `humata` MCP server entry can never be deleted or hijacked
//!   by any rewrite stage.

pub mod compose;
pub mod model;
pub mod rewrite;

pub use compose::{
    ComposeError, ConfigComposer, MaterializeReport, SanitizedConfigView, SessionEnvPlan,
};
pub use model::{
    AgentConfigSpec, AgentKind, ConfigFormat, EffectiveSyncPolicy, MachineSyncOverride,
    SecretDenylist, SkillsMode, SyncCategory, SyncConfig, SyncToggles,
};
pub use rewrite::{
    BashFilterConfig, DeclarativeRules, McpServerRule, PathRewriteRule, PipelineReport,
    ProtectedHumataMcp, RewriteContext, RewriteError, RewritePipeline, RewriteViolation,
    SchemeScriptConfig,
};
