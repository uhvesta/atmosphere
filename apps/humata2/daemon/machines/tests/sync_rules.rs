//! Scripted sync rule config (TS-3, TS-4).

use std::fs;
use std::path::PathBuf;

use humata2_machines::{parse_rules, RuleProblem, RuleSource, SyncRulesError};
use tempfile::TempDir;

fn problems(text: &str) -> Vec<RuleProblem> {
    match parse_rules(text) {
        Err(SyncRulesError::Invalid { problems }) => problems,
        other => panic!("expected problems, got {other:?}"),
    }
}

#[test]
fn a_folder_rule_parses_with_all_fields() {
    let rules = parse_rules(
        r#"
[rules.dotfiles]
source = "/host/dotfiles"
destination = "~/.config/tools"
before = "echo before {name}"
after = "echo after"
exclude = ["*.bak", "cache/**"]
"#,
    )
    .unwrap();
    assert_eq!(rules.len(), 1);
    let rule = &rules[0];
    assert_eq!(rule.name, "dotfiles");
    assert_eq!(rule.source, RuleSource::Folder(PathBuf::from("/host/dotfiles")));
    assert_eq!(rule.destination, PathBuf::from(".config/tools"));
    assert!(rule.before.is_some() && rule.after.is_some());
    assert_eq!(rule.exclude, ["*.bak", "cache/**"]);
}

#[test]
fn a_command_rule_parses_and_relative_destinations_are_under_home() {
    let rules = parse_rules("[rules.gen]\ncommand = \"make-tools\"\ndestination = \"tools/gen\"\n").unwrap();
    assert_eq!(rules[0].source, RuleSource::Command("make-tools".into()));
    assert_eq!(rules[0].destination, PathBuf::from("tools/gen"));
}

#[test]
fn exactly_one_of_source_and_command() {
    let both = problems("[rules.a]\nsource = \"/x\"\ncommand = \"y\"\ndestination = \"~/a\"\n");
    assert!(matches!(both.as_slice(), [RuleProblem::BothSources { rule }] if rule == "a"));
    let neither = problems("[rules.a]\ndestination = \"~/a\"\n");
    assert!(matches!(neither.as_slice(), [RuleProblem::NoSource { .. }]));
}

#[test]
fn skills_is_reserved() {
    let found = problems("[rules.skills]\nsource = \"/x\"\ndestination = \"~/a\"\n");
    assert!(found.iter().any(|p| matches!(p, RuleProblem::ReservedName { .. })));
}

#[test]
fn destinations_outside_home_or_in_managed_folders_are_refused() {
    for destination in [
        "/etc/tools",
        "/home/other/x",
        "~/../x",
        "../x",
        "a/../../x",
        "~",
        "",
        "~/Humata/.skills",
        "~/Humata/.skills/claude",
        "Humata/.skills-sets/x",
        "~/.humata2",
        "~/.humata2/state",
        "~other/x",
    ] {
        let text = format!("[rules.a]\nsource = \"/x\"\ndestination = {destination:?}\n");
        let found = problems(&text);
        assert!(
            found.iter().any(|p| matches!(p, RuleProblem::BadDestination { .. } | RuleProblem::NoDestination { .. })),
            "{destination:?} should be refused, got {found:?}"
        );
    }
}

#[test]
fn a_destination_beside_the_managed_folders_is_allowed() {
    let rules = parse_rules("[rules.a]\nsource = \"/x\"\ndestination = \"~/Humata/other\"\n").unwrap();
    assert_eq!(rules[0].destination, PathBuf::from("Humata/other"));
}

#[test]
fn every_problem_across_rules_is_reported_together() {
    let found = problems(
        "[rules.a]\nsource = \"/x\"\ndestination = \"/etc\"\n[rules.b]\ncommand = \"\"\ndestination = \"~/b\"\n",
    );
    assert!(found.len() >= 2);
}

#[test]
fn unknown_fields_and_bad_toml_are_syntax_errors() {
    assert!(matches!(parse_rules("[rules.a]\nsorce = \"x\"\n"), Err(SyncRulesError::Syntax { .. })));
    assert!(matches!(parse_rules("[rules"), Err(SyncRulesError::Syntax { .. })));
}

#[test]
fn a_folder_source_scans_with_secret_filter_and_excludes() {
    let dir = TempDir::new().unwrap();
    for (path, body) in [("tool/run.sh", "x"), ("tool/.env", "SECRET=1"), ("tool/old.bak", "x"), ("top.txt", "y")] {
        let full = dir.path().join(path);
        fs::create_dir_all(full.parent().unwrap()).unwrap();
        fs::write(full, body).unwrap();
    }
    let text = format!(
        "[rules.tools]\nsource = {:?}\ndestination = \"~/tools\"\nexclude = [\"*.bak\"]\n",
        dir.path().to_str().unwrap()
    );
    let rules = parse_rules(&text).unwrap();
    let set = rules[0].scan_source().unwrap();
    let paths: Vec<_> = set.entries().iter().map(|e| e.path.as_str()).collect();
    assert_eq!(paths, ["tools/tool/run.sh", "tools/top.txt"]);
    assert_eq!(set.excluded_files(), 2);
}

#[test]
fn a_command_source_is_not_scanned() {
    let rules = parse_rules("[rules.a]\ncommand = \"gen\"\ndestination = \"~/a\"\n").unwrap();
    assert!(matches!(rules[0].scan_source(), Err(SyncRulesError::NotAFolder { .. })));
}
