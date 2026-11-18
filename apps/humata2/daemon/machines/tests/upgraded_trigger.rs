//! The `upgraded` trigger (D689): a rule names it, it applies only to it,
//! and it isn't a session trigger.

use humata2_machines::{parse_machine_scripts, ScriptName, Trigger};

fn present(_: &ScriptName) -> bool {
    true
}

#[test]
fn an_upgraded_rule_applies_only_after_an_upgrade() {
    let text = "[[rule]]\nname = \"restart\"\non = \"upgraded\"\n[[rule.step]]\nsend = \"/restart\"\n";
    let rules = parse_machine_scripts(text, &present, &["devbox".to_owned()]).unwrap().rules;
    assert_eq!(rules.len(), 1);
    assert!(rules[0].applies(Trigger::Upgraded, "devbox"));
    assert!(!rules[0].applies(Trigger::Connected, "devbox"));
    assert!(!rules[0].applies(Trigger::Added, "devbox"));
}

#[test]
fn upgraded_round_trips_and_is_not_about_a_session() {
    assert_eq!(Trigger::parse(Trigger::Upgraded.as_str()), Some(Trigger::Upgraded));
    assert!(!Trigger::Upgraded.is_session());
}
