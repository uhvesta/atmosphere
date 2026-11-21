use std::fmt::Write;

use crate::IntentSpec;

pub fn render_milestone_brief(spec: &IntentSpec, version_id: Option<&str>) -> String {
    let mut out = String::new();

    let _ = writeln!(out, "# Milestone Brief: {}", spec.metadata.title);
    let _ = writeln!(out);
    let _ = writeln!(out, "- **Intent ID**: `{}`", spec.metadata.id);
    let _ = writeln!(out, "- **Channel**: `{}`", spec.metadata.channel);
    let _ = writeln!(out, "- **Status**: {}", spec.metadata.status.label());
    if let Some(ver) = version_id {
        let _ = writeln!(out, "- **Pinned Version**: `{ver}`");
    } else if let Some(ver) = spec.metadata.approved_doc_version.as_deref() {
        let _ = writeln!(out, "- **Pinned Version**: `{ver}`");
    }
    if !spec.metadata.repos.is_empty() {
        let _ = writeln!(out, "- **Repos**: {}", spec.metadata.repos.join(", "));
    }

    let _ = writeln!(out);
    let _ = writeln!(out, "## Outcome");
    let _ = writeln!(out, "{}", spec.outcome.trim());

    if !spec.non_goals.is_empty() {
        let _ = writeln!(out);
        let _ = writeln!(out, "## Non-Goals");
        for ng in &spec.non_goals {
            let _ = writeln!(out, "- {ng}");
        }
    }

    if !spec.invariants.is_empty() {
        let _ = writeln!(out);
        let _ = writeln!(out, "## Invariants");
        for inv in &spec.invariants {
            let _ = writeln!(out, "- {inv}");
        }
    }

    if !spec.constraints.forbidden_actions.is_empty() {
        let _ = writeln!(out);
        let _ = writeln!(out, "## Forbidden Actions");
        for act in &spec.constraints.forbidden_actions {
            let _ = writeln!(out, "- {act}");
        }
    }

    if !spec.constraints.approval_boundaries.is_empty() {
        let _ = writeln!(out);
        let _ = writeln!(out, "## Approval Boundaries");
        for b in &spec.constraints.approval_boundaries {
            let _ = writeln!(out, "- {b}");
        }
    }

    if !spec.steps.is_empty() {
        let _ = writeln!(out);
        let _ = writeln!(out, "## Implementation Steps");
        for (idx, step) in spec.steps.iter().enumerate() {
            let _ = writeln!(out, "{}. **{}** (`{}`)", idx + 1, step.title, step.id);
            let _ = writeln!(out, "   - Outcome: {}", step.outcome.trim());
            if !step.depends_on.is_empty() {
                let _ = writeln!(out, "   - Depends on: {}", step.depends_on.join(", "));
            }
            if !step.crates.is_empty() {
                let _ = writeln!(out, "   - Crates: {}", step.crates.join(", "));
            }
            let _ = writeln!(out, "   - Verify: `{}`", step.verify.trim());
        }
    }

    if !spec.acceptance.is_empty() {
        let _ = writeln!(out);
        let _ = writeln!(out, "## Acceptance Criteria");
        for acc in &spec.acceptance {
            let _ = writeln!(out, "- **{}**", acc.id);
            let _ = writeln!(out, "  - Given: {}", acc.given.trim());
            let _ = writeln!(out, "  - When: {}", acc.when.trim());
            let _ = writeln!(out, "  - Then: {}", acc.then.trim());
        }
    }

    out
}
