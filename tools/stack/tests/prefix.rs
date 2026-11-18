//! The first part of the branch names: a one-off `--prefix`, else
//! stack.toml's `prefix`, else the gh login. Open pull requests keep their
//! branches.

mod support;

use serde_json::Value;
use support::{World, history_rows, message};

/// Topic `codec`, and topic `journal` relative to it.
fn two_topics() -> World {
    let world = World::new();
    world.commit("codec.rs", "codec v1\n", &message("add the codec", "Topic: codec"));
    world.commit(
        "journal.rs",
        "journal v1\n",
        &message("compress the journal", "Topic: journal\nRelative: codec"),
    );
    world
}

fn status_topics(world: &World) -> Vec<Value> {
    let run = world.stack(&["status", "--json"]);
    run.ok();
    let printed: Value = serde_json::from_str(&run.stdout).expect("status --json prints JSON");
    printed["topics"].as_array().cloned().expect("topics")
}

#[test]
fn with_nothing_set_the_gh_login_is_used() {
    let world = two_topics();
    world.edit_state(|state| state["login"] = Value::from("carol"));
    world.stack(&["upload"]).ok();

    assert_eq!(world.pr("carol/stack/journal")["base"], "carol/stack/codec");
    assert!(world.remote_branch("carol/stack/codec").is_some());
    assert!(world.remote_branch("alice/stack/codec").is_none());
}

#[test]
fn the_files_prefix_beats_the_login() {
    let world = two_topics();
    world.config("stack.toml", "prefix = \"foo\"\n");
    world.stack(&["upload"]).ok();

    assert_eq!(world.prs().len(), 2, "{:#?}", world.prs());
    assert_eq!(world.pr("foo/stack/codec")["base"], "main");
    assert_eq!(world.pr("foo/stack/journal")["base"], "foo/stack/codec");
    assert!(world.remote_branch("foo/stack/journal").is_some());
    assert!(world.remote_branch("alice/stack/codec").is_none());
}

#[test]
fn the_prefix_flag_beats_the_file_for_one_run() {
    let world = two_topics();
    world.config("stack.toml", "prefix = \"foo\"\n");
    world.stack(&["upload", "--prefix", "bar"]).ok();

    assert_eq!(world.pr("bar/stack/journal")["base"], "bar/stack/codec");
    assert!(world.remote_branch("foo/stack/codec").is_none());
}

#[test]
fn a_prefix_change_leaves_an_open_prs_branch_alone() {
    let world = two_topics();
    world.stack(&["upload"]).ok();
    let journal_number = world.pr("alice/stack/journal")["number"].clone();

    world.config("stack.toml", "prefix = \"foo\"\n");
    world.amend("journal.rs", "journal v2\n");
    world.commit(
        "index.rs",
        "index v1\n",
        &message("index the journal", "Topic: index\nRelative: journal"),
    );
    world.stack(&["upload"]).ok();

    // The amend updated the same PR on its old branch.
    assert_eq!(world.prs().len(), 3, "{:#?}", world.prs());
    let journal = world.pr("alice/stack/journal");
    assert_eq!(journal["number"], journal_number);
    assert_eq!(history_rows(&journal).len(), 2);
    let head = world.remote_branch("alice/stack/journal").expect("journal branch");
    assert_eq!(world.remote_git(&["show", &format!("{head}:journal.rs")]), "journal v2");
    assert!(world.remote_branch("foo/stack/journal").is_none());
    assert!(world.remote_branch("foo/stack/codec").is_none());

    // Only the new topic uses the new prefix, on top of the old branch.
    assert_eq!(world.pr("foo/stack/index")["base"], "alice/stack/journal");

    // Status reads the same branches, and nothing needs another upload.
    let topics = status_topics(&world);
    let branches: Vec<&str> = topics.iter().filter_map(|topic| topic["branch"].as_str()).collect();
    assert_eq!(branches, ["alice/stack/codec", "alice/stack/journal", "foo/stack/index"]);
    assert!(topics.iter().all(|topic| topic["needs_upload"] == false), "{topics:#?}");
}

#[test]
fn a_bad_prefix_is_refused_before_anything_is_pushed() {
    for bad in ["two words", "a/b", "dots..here", "x.lock", ""] {
        let world = two_topics();
        world.config("stack.toml", &format!("prefix = {bad:?}\n"));
        let run = world.stack(&["upload"]);
        assert_eq!(run.code, 2, "prefix {bad:?}\nstdout:\n{}\nstderr:\n{}", run.stdout, run.stderr);
        assert!(world.prs().is_empty(), "prefix {bad:?}: {:#?}", world.prs());
        assert_eq!(world.remote_git(&["for-each-ref", "--format=%(refname)", "refs/heads/"]), "refs/heads/main");
    }

    let world = two_topics();
    let run = world.stack(&["upload", "--prefix", "a/b"]);
    assert_eq!(run.code, 2, "stderr:\n{}", run.stderr);
    assert!(world.prs().is_empty());
}
