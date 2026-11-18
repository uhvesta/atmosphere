//! `stack check`: local only, never calls gh.

mod support;

use serde_json::Value;
use support::{World, message};

#[test]
fn check_prints_the_plan_and_never_calls_github() {
    let world = World::new();
    world.commit("codec.rs", "codec\n", &message("add the codec", "Topic: codec"));
    world.commit("journal.rs", "journal\n", &message("compress the journal", "Topic: journal\nRelative: codec"));
    world.commit("notes.txt", "notes\n", "wip: notes");

    let run = world.stack(&["check"]);
    run.ok();
    for expected in ["codec", "journal", "wip: notes"] {
        assert!(run.stdout.contains(expected), "missing {expected}: {}", run.stdout);
    }
    assert!(world.calls().is_empty(), "{:#?}", world.calls());
}

#[test]
fn check_json_lists_topics_in_order_with_their_bases() {
    let world = World::new();
    world.commit("codec.rs", "codec\n", &message("add the codec", "Topic: codec"));
    world.commit("journal.rs", "journal\n", &message("compress the journal", "Topic: journal\nRelative: codec"));
    world.commit("notes.txt", "notes\n", "wip: notes");

    let run = world.stack(&["check", "--json"]);
    run.ok();
    let plan: Value = serde_json::from_str(&run.stdout).expect("check --json prints JSON");
    let topics = plan["topics"].as_array().expect("topics");
    assert_eq!(topics.len(), 2);
    assert_eq!(topics[0]["topic"], "codec");
    assert_eq!(topics[0]["relative"], Value::Null);
    assert_eq!(topics[1]["topic"], "journal");
    assert_eq!(topics[1]["relative"], "codec");
    assert_eq!(plan["untagged"].as_array().map(Vec::len), Some(1));
}

#[test]
fn a_relative_loop_is_a_usage_error() {
    let world = World::new();
    world.commit("a.rs", "a\n", &message("add a", "Topic: a\nRelative: b"));
    world.commit("b.rs", "b\n", &message("add b", "Topic: b\nRelative: a"));
    let run = world.stack(&["check"]);
    assert_eq!(run.code, 2, "stderr:\n{}", run.stderr);
}

#[test]
fn an_unknown_relative_is_a_usage_error() {
    let world = World::new();
    world.commit("a.rs", "a\n", &message("add a", "Topic: a\nRelative: nowhere"));
    let run = world.stack(&["check"]);
    assert_eq!(run.code, 2, "stderr:\n{}", run.stderr);
    assert!(run.stderr.contains("nowhere"), "{}", run.stderr);
}

#[test]
fn a_missing_required_section_is_a_usage_error() {
    let world = World::new();
    world.commit("a.rs", "a\n", "add a\n\nJust a summary.\n\nTopic: a\n");
    let run = world.stack(&["check"]);
    assert_eq!(run.code, 2, "stderr:\n{}", run.stderr);
}

#[test]
fn a_commit_that_needs_another_topic_is_a_conflict() {
    let world = World::new();
    world.commit("shared.rs", "one\n", &message("add shared", "Topic: base"));
    // Edits the line the first topic added, but claims to stand alone.
    world.commit("shared.rs", "two\n", &message("change shared", "Topic: other"));
    let run = world.stack(&["check"]);
    assert_eq!(run.code, 3, "stdout:\n{}\nstderr:\n{}", run.stdout, run.stderr);
}
