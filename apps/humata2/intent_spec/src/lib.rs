mod brief;
mod diff;
mod validate;
mod yaml;

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub use diff::IntentDiff;

pub const SCHEMA_VERSION: &str = "humata.intent/v1";

pub fn is_intent_spec(path: &str) -> bool {
    let base = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let lower = base.to_ascii_lowercase();
    lower == "intent.yaml"
        || (lower.ends_with(".intent.yaml") && lower.len() > ".intent.yaml".len())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IntentSpec {
    pub schema: String,
    pub metadata: Metadata,
    pub outcome: String,
    #[serde(default)]
    pub non_goals: Vec<String>,
    #[serde(default)]
    pub invariants: Vec<String>,
    #[serde(default)]
    pub constraints: Constraints,
    #[serde(default)]
    pub touch_points: Vec<TouchPoint>,
    #[serde(default)]
    pub steps: Vec<Step>,
    #[serde(default)]
    pub acceptance: Vec<Acceptance>,
    #[serde(default)]
    pub open_questions: Vec<OpenQuestion>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Metadata {
    pub id: String,
    pub title: String,
    pub status: IntentStatus,
    pub channel: String,
    #[serde(default)]
    pub todo_key: Option<String>,
    #[serde(default)]
    pub repos: Vec<String>,
    #[serde(default)]
    pub approved_doc_version: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IntentStatus {
    Draft,
    Review,
    Approved,
    Implementing,
    Verified,
}

impl IntentStatus {
    pub const fn label(self) -> &'static str {
        match self {
            IntentStatus::Draft => "Draft",
            IntentStatus::Review => "Review",
            IntentStatus::Approved => "Approved",
            IntentStatus::Implementing => "Implementing",
            IntentStatus::Verified => "Verified",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Constraints {
    #[serde(default)]
    pub forbidden_actions: Vec<String>,
    #[serde(default)]
    pub approval_boundaries: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TouchPoint {
    pub repo: String,
    pub path: String,
    #[serde(default)]
    pub mode: TouchMode,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TouchMode {
    #[default]
    Read,
    Modify,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub id: String,
    pub title: String,
    pub outcome: String,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub crates: Vec<String>,
    pub verify: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Acceptance {
    pub id: String,
    pub given: String,
    pub when: String,
    pub then: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenQuestion {
    pub id: String,
    pub question: String,
    #[serde(default)]
    pub default_choice: Option<String>,
    #[serde(default)]
    pub blocking: bool,
    #[serde(default)]
    pub resolved: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum ParseError {
    #[error("YAML parse error at line {line}: {message}")]
    Yaml { line: usize, message: String },
    #[error("Schema error: {0}")]
    Schema(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationIssue {
    pub path: String,
    pub message: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationReport {
    pub errors: Vec<ValidationIssue>,
    pub blocking_open_questions: Vec<String>,
}

impl ValidationReport {
    pub fn is_schema_valid(&self) -> bool {
        self.errors.is_empty()
    }

    pub fn can_approve(&self) -> bool {
        self.errors.is_empty() && self.blocking_open_questions.is_empty()
    }
}

pub fn parse(yaml: &str) -> Result<IntentSpec, ParseError> {
    let val = yaml::parse_yaml_to_value(yaml)?;
    serde_json::from_value(val).map_err(|e| ParseError::Schema(e.to_string()))
}

pub fn validate(spec: &IntentSpec) -> ValidationReport {
    validate::validate_spec(spec)
}

pub fn parse_and_validate(yaml: &str) -> Result<(IntentSpec, ValidationReport), ParseError> {
    let spec = parse(yaml)?;
    let report = validate(&spec);
    Ok((spec, report))
}

impl IntentSpec {
    pub fn unresolved_open_questions(&self) -> usize {
        self.open_questions.iter().filter(|q| !q.resolved).count()
    }

    pub fn blocking_open_questions(&self) -> Vec<&OpenQuestion> {
        self.open_questions
            .iter()
            .filter(|q| q.blocking && !q.resolved)
            .collect()
    }

    pub fn to_milestone_brief(&self, version_id: Option<&str>) -> String {
        brief::render_milestone_brief(self, version_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_YAML: &str = r##"---
schema: humata.intent/v1
metadata:
  id: intent-001
  title: Add Intent Spec Support
  status: review
  channel: "#platform"
  todo_key: HUM-42
  repos: [atmosphere, humata2]
outcome: |
  Enable structured intent specs that gate milestone execution until approved.
non_goals:
  - Automatic code generation without review
invariants:
  - Every approved spec has zero blocking open questions
constraints:
  forbidden_actions:
    - Force-pushing to main
  approval_boundaries:
    - Schema changes require owner approval
touch_points:
  - repo: atmosphere
    path: apps/humata2/intent_spec/src/lib.rs
    mode: modify
    note: Core parser and validator
  - repo: atmosphere
    path: apps/humata2/todo/BUILD.bazel
steps:
  - id: s1
    title: Implement parser
    outcome: Strict YAML 1.2 subset parser
    crates: [humata2_intent_spec]
    verify: bazel test //apps/humata2/intent_spec:intent_spec_unit_test
  - id: s2
    title: Wire UI
    outcome: Render spec card in channel
    depends_on: [s1]
    crates: [humata2_web]
    verify: bazel build //apps/humata2/web:all
acceptance:
  - id: ac1
    given: A valid intent.yaml document
    when: The user clicks Approve & Start Milestone
    then: A milestone brief is generated with the pinned version
open_questions:
  - id: q1
    question: Should draft specs show a warning banner?
    default_choice: Yes
    blocking: false
    resolved: false
...
"##;

    #[test]
    fn parses_and_validates_valid_spec() {
        let (spec, report) = parse_and_validate(VALID_YAML).expect("should parse");
        assert!(report.is_schema_valid(), "errors: {:?}", report.errors);
        assert!(report.can_approve());
        assert_eq!(spec.metadata.id, "intent-001");
        assert_eq!(spec.metadata.status, IntentStatus::Review);
        assert_eq!(spec.metadata.status.label(), "Review");
        assert_eq!(spec.touch_points[0].mode, TouchMode::Modify);
        assert_eq!(spec.touch_points[1].mode, TouchMode::Read);
        assert_eq!(spec.unresolved_open_questions(), 1);
        assert!(spec.blocking_open_questions().is_empty());
    }

    #[test]
    fn rejects_unknown_fields() {
        let yaml = format!("{VALID_YAML}\nextra_field: nope\n");
        let yaml = yaml.replace("...", "");
        let err = parse(&yaml).unwrap_err();
        assert!(matches!(err, ParseError::Schema(_)), "got: {err:?}");
    }

    #[test]
    fn rejects_duplicate_keys_with_line_number() {
        let yaml = r#"
schema: humata.intent/v1
schema: humata.intent/v1
metadata:
  id: x
  title: y
  status: draft
  channel: c
outcome: o
steps:
  - id: s1
    title: t
    outcome: o
    verify: v
"#;
        let err = parse(yaml).unwrap_err();
        match err {
            ParseError::Yaml { line, message } => {
                assert_eq!(line, 3);
                assert!(message.contains("duplicate key"));
            }
            other => panic!("expected Yaml error, got {other:?}"),
        }
    }

    #[test]
    fn rejects_missing_required_fields() {
        let yaml = r#"
schema: humata.intent/v1
metadata:
  id: x
  title: y
  status: draft
  channel: c
"#;
        let err = parse(yaml).unwrap_err();
        assert!(matches!(err, ParseError::Schema(_)));
    }

    #[test]
    fn detects_wrong_schema_version() {
        let yaml = VALID_YAML.replace("humata.intent/v1", "humata.intent/v99");
        let (_, report) = parse_and_validate(&yaml).unwrap();
        assert!(!report.is_schema_valid());
        assert!(report.errors.iter().any(|e| e.path == "schema"));
    }

    #[test]
    fn detects_missing_dep_and_self_cycle() {
        let yaml = r#"
schema: humata.intent/v1
metadata:
  id: id1
  title: Title
  status: draft
  channel: ch
outcome: Do thing
steps:
  - id: s1
    title: Step 1
    outcome: Out 1
    depends_on: [s1, missing_step]
    verify: echo ok
"#;
        let (_, report) = parse_and_validate(yaml).unwrap();
        assert!(!report.is_schema_valid());
        assert!(report
            .errors
            .iter()
            .any(|e| e.message.contains("cannot depend on itself")));
        assert!(report
            .errors
            .iter()
            .any(|e| e.message.contains("unknown step dependency `missing_step`")));
    }

    #[test]
    fn detects_two_node_and_three_node_cycles() {
        let two_node = r#"
schema: humata.intent/v1
metadata:
  id: id1
  title: Title
  status: draft
  channel: ch
outcome: Do thing
steps:
  - id: a
    title: A
    outcome: Out A
    depends_on: [b]
    verify: echo a
  - id: b
    title: B
    outcome: Out B
    depends_on: [a]
    verify: echo b
"#;
        let (_, report) = parse_and_validate(two_node).unwrap();
        assert!(!report.is_schema_valid());
        assert!(report
            .errors
            .iter()
            .any(|e| e.message.contains("dependency cycle")));

        let three_node = r#"
schema: humata.intent/v1
metadata:
  id: id1
  title: Title
  status: draft
  channel: ch
outcome: Do thing
steps:
  - id: a
    title: A
    outcome: Out A
    depends_on: [b]
    verify: echo a
  - id: b
    title: B
    outcome: Out B
    depends_on: [c]
    verify: echo b
  - id: c
    title: C
    outcome: Out C
    depends_on: [a]
    verify: echo c
"#;
        let (_, report3) = parse_and_validate(three_node).unwrap();
        assert!(!report3.is_schema_valid());
        assert!(report3
            .errors
            .iter()
            .any(|e| e.message.contains("dependency cycle")));
    }

    #[test]
    fn blocking_open_questions_gate_can_approve() {
        let yaml = VALID_YAML.replace("blocking: false", "blocking: true");
        let (spec, report) = parse_and_validate(&yaml).unwrap();
        assert!(report.is_schema_valid());
        assert!(!report.can_approve());
        assert_eq!(report.blocking_open_questions, vec!["q1".to_string()]);
        assert_eq!(spec.blocking_open_questions().len(), 1);
    }

    #[test]
    fn computes_diff_and_summary_lines() {
        let before = parse(VALID_YAML).unwrap();
        let mut after = before.clone();
        assert!(IntentDiff::compute(&before, &after).is_empty());

        after.metadata.status = IntentStatus::Approved;
        after.outcome = "Updated outcome".to_string();
        after.non_goals.push("New non-goal".to_string());
        after.steps[0].title = "Updated parser".to_string();
        after.open_questions[0].resolved = true;

        let diff = IntentDiff::compute(&before, &after);
        assert!(!diff.is_empty());
        assert!(diff.outcome_changed);
        assert_eq!(
            diff.status_changed,
            Some((IntentStatus::Review, IntentStatus::Approved))
        );
        assert_eq!(diff.added_non_goals, vec!["New non-goal"]);
        assert_eq!(diff.modified_steps, vec!["s1"]);
        assert_eq!(diff.resolved_questions, vec!["q1"]);

        let lines = diff.summary_lines();
        assert!(lines.iter().any(|l| l.contains("Review -> Approved")));
        assert!(lines.iter().any(|l| l.contains("Outcome updated")));
        assert!(lines.iter().any(|l| l.contains("Modified steps: s1")));
        assert!(lines.iter().any(|l| l.contains("Resolved questions: q1")));
    }

    #[test]
    fn renders_milestone_brief_with_version() {
        let spec = parse(VALID_YAML).unwrap();
        let brief = spec.to_milestone_brief(Some("v3"));
        assert!(brief.contains("Add Intent Spec Support"));
        assert!(brief.contains("intent-001"));
        assert!(brief.contains("#platform"));
        assert!(brief.contains("`v3`"));
        assert!(brief.contains("Force-pushing to main"));
        assert!(brief.contains("Schema changes require owner approval"));
        assert!(brief.contains("bazel test //apps/humata2/intent_spec:intent_spec_unit_test"));
        assert!(brief.contains("Given: A valid intent.yaml document"));
    }

    #[test]
    fn is_intent_spec_matches_paths() {
        assert!(is_intent_spec("intent.yaml"));
        assert!(is_intent_spec("INTENT.YAML"));
        assert!(is_intent_spec("docs/auth.intent.yaml"));
        assert!(is_intent_spec("C:\\repo\\feature.INTENT.YAML"));
        assert!(!is_intent_spec("intent.yml"));
        assert!(!is_intent_spec("my-intent.yaml"));
        assert!(!is_intent_spec("notes.md"));
    }
}
