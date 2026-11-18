//! `stack status` after uploads.

mod support;

use serde_json::Value;
use support::{World, message};

fn status(world: &World) -> Vec<Value> {
    let run = world.stack(&["status", "--json"]);
    run.ok();
    let printed: Value = serde_json::from_str(&run.stdout).expect("status --json prints JSON");
    printed["topics"].as_array().cloned().expect("topics")
}

#[test]
fn status_names_each_pr_and_whether_it_needs_an_upload() {
    let world = World::new();
    world.commit("codec.rs", "codec\n", &message("add the codec", "Topic: codec"));
    world.commit("journal.rs", "journal\n", &message("compress the journal", "Topic: journal\nRelative: codec"));

    let before = status(&world);
    assert!(before.iter().all(|topic| topic["number"].is_null()), "{before:#?}");
    assert!(before.iter().all(|topic| topic["needs_upload"] == true), "{before:#?}");

    world.stack(&["upload"]).ok();
    let after = status(&world);
    assert_eq!(after.len(), 2);
    assert_eq!(after[0]["topic"], "codec");
    assert_eq!(after[0]["number"], world.pr("alice/stack/codec")["number"]);
    assert_eq!(after[1]["base"], "alice/stack/codec");
    assert!(after.iter().all(|topic| topic["needs_upload"] == false), "{after:#?}");

    world.amend("journal.rs", "journal v2\n");
    let amended = status(&world);
    assert_eq!(amended[0]["needs_upload"], false);
    assert_eq!(amended[1]["needs_upload"], true);
}

#[test]
fn status_lists_open_stack_prs_no_topic_uses() {
    let world = World::new();
    world.commit("codec.rs", "codec\n", &message("add the codec", "Topic: codec"));
    world.commit("old.rs", "old\n", &message("an old idea", "Topic: old-idea\nRelative: codec"));
    world.stack(&["upload"]).ok();
    world.git(&["reset", "--quiet", "--hard", "HEAD~1"]);

    let run = world.stack(&["status", "--json"]);
    run.ok();
    let printed: Value = serde_json::from_str(&run.stdout).expect("status --json prints JSON");
    let orphaned = printed["orphaned"].as_array().cloned().unwrap_or_default();
    assert_eq!(orphaned.len(), 1, "{printed:#}");
    assert_eq!(orphaned[0]["branch"], "alice/stack/old-idea");
    assert_eq!(orphaned[0]["number"], world.pr("alice/stack/old-idea")["number"]);
}

#[test]
fn status_draws_the_chain_from_the_relatives() {
    let world = World::new();
    world.commit("codec.rs", "codec\n", &message("add the codec", "Topic: codec"));
    world.commit("journal.rs", "journal\n", &message("compress the journal", "Topic: journal\nRelative: codec"));
    world.commit("docs.md", "docs\n", &message("document it", "Topic: docs"));

    let run = world.stack(&["status", "--json"]);
    run.ok();
    let printed: Value = serde_json::from_str(&run.stdout).expect("status --json prints JSON");
    let topics = printed["topics"].as_array().cloned().expect("topics");
    let of = |name: &str| topics.iter().find(|topic| topic["topic"] == name).cloned().expect("topic");
    assert_eq!((of("codec")["parent"].clone(), of("codec")["depth"].clone()), (Value::Null, Value::from(0)));
    assert_eq!((of("journal")["parent"].clone(), of("journal")["depth"].clone()), (Value::from("codec"), Value::from(1)));
    assert_eq!(of("docs")["depth"], 0);
    assert!(printed["restack"].is_null(), "{printed:#}");
}
