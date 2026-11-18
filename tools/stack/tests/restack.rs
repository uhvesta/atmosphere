//! `stack restack`: stop at an earlier topic, amend it, replay the rest.

mod support;

use serde_json::Value;
use support::{World, history_rows, message};

/// Topics codec, journal and index, each on the one before, each in its own
/// file; `shared.txt` is edited by codec and journal so an amended codec
/// conflicts with journal.
fn three_topics() -> (World, [String; 3]) {
    let world = World::new();
    world.write("shared.txt", "base\n");
    world.git(&["add", "shared.txt"]);
    world.git(&["commit", "--quiet", "-m", "shared file"]);
    world.git(&["push", "--quiet", "origin", "HEAD:refs/heads/main"]);
    world.git(&["fetch", "--quiet", "origin"]);
    world.write("shared.txt", "codec\n");
    let codec = world.commit("codec.rs", "codec v1\n", &message("add the codec", "Topic: codec"));
    world.write("shared.txt", "journal\n");
    let journal = world.commit(
        "journal.rs",
        "journal v1\n",
        &message("compress the journal", "Topic: journal\nRelative: codec"),
    );
    let index = world.commit(
        "index.rs",
        "index v1\n",
        &message("add the index", "Topic: index\nRelative: journal"),
    );
    (world, [codec, journal, index])
}

fn subjects(world: &World) -> Vec<String> {
    world
        .git(&["log", "--format=%s", "origin/main..HEAD"])
        .lines()
        .map(str::to_owned)
        .collect()
}

fn rebasing(world: &World) -> bool {
    world.work.join(".git/rebase-merge").exists()
}

fn head(world: &World) -> String {
    world.git(&["rev-parse", "HEAD"])
}

fn head_subject(world: &World) -> String {
    world.git(&["log", "-1", "--format=%s"])
}

fn restack_status(world: &World) -> Value {
    let run = world.stack(&["status", "--json"]);
    run.ok();
    let printed: Value = serde_json::from_str(&run.stdout).expect("status --json prints JSON");
    printed["restack"].clone()
}

const ALL: [&str; 3] = ["add the index", "compress the journal", "add the codec"];

#[test]
fn editing_a_topic_replays_the_later_topics_onto_the_amendment() {
    let (world, _) = three_topics();
    world.stack(&["upload"]).ok();
    for branch in ["alice/stack/codec", "alice/stack/journal", "alice/stack/index"] {
        assert_eq!(history_rows(&world.pr(branch)).len(), 1);
    }

    world.stack(&["restack", "edit", "--topic", "codec"]).ok();
    assert_eq!(head_subject(&world), "add the codec");
    world.amend("codec.rs", "codec v2\n");
    world.stack(&["restack", "continue"]).ok();

    assert_eq!(subjects(&world), ALL);
    assert!(!rebasing(&world));
    assert!(!world.work.join(".git/stack/restack.json").exists());
    assert_eq!(world.git(&["show", "HEAD:codec.rs"]), "codec v2");
    assert_eq!(world.git(&["show", "HEAD^:codec.rs"]), "codec v2");
    assert_eq!(world.git(&["show", "HEAD:journal.rs"]), "journal v1");
    assert_eq!(restack_status(&world), Value::Null);

    world.stack(&["upload"]).ok();
    for branch in ["alice/stack/codec", "alice/stack/journal", "alice/stack/index"] {
        assert_eq!(history_rows(&world.pr(branch)).len(), 2, "{branch}");
    }
}

#[test]
fn editing_a_commit_by_sha_stops_there() {
    let (world, [_, journal, _]) = three_topics();
    world.stack(&["restack", "edit", "--commit", &journal[..10]]).ok();
    assert_eq!(head_subject(&world), "compress the journal");
    world.amend("journal.rs", "journal v2\n");
    world.stack(&["restack", "continue"]).ok();

    assert_eq!(subjects(&world), ALL);
    assert_eq!(world.git(&["show", "HEAD:journal.rs"]), "journal v2");
    assert_eq!(world.git(&["show", "HEAD:index.rs"]), "index v1");
    assert!(!rebasing(&world));
}

#[test]
fn step_stops_after_each_later_topic() {
    let (world, _) = three_topics();
    world.stack(&["restack", "edit", "--topic", "codec", "--step"]).ok();
    world.amend("codec.rs", "codec v2\n");

    world.stack(&["restack", "continue", "--step"]).ok();
    assert_eq!(head_subject(&world), "compress the journal");
    assert!(rebasing(&world));

    world.stack(&["restack", "continue", "--step"]).ok();
    assert_eq!(head_subject(&world), "add the index");
    assert!(rebasing(&world));

    world.stack(&["restack", "continue"]).ok();
    assert!(!rebasing(&world));
    assert_eq!(subjects(&world), ALL);
}

#[test]
fn a_failing_exec_stops_the_restack_until_it_is_fixed() {
    let (world, _) = three_topics();
    world
        .stack(&["restack", "edit", "--topic", "codec", "--exec", "test -f marker"])
        .ok();
    let run = world.stack(&["restack", "continue"]);
    assert_eq!(run.code, 5, "{}", run.stderr);
    assert_eq!(head_subject(&world), "compress the journal");
    assert!(rebasing(&world));

    world.write("marker", "here\n");
    world.stack(&["restack", "continue"]).ok();
    assert!(!rebasing(&world));
    assert_eq!(subjects(&world), ALL);
}

/// An amended codec that conflicts with journal, which edits the same line.
fn conflicting() -> (World, String) {
    let (world, _) = three_topics();
    let original = head(&world);
    world.stack(&["restack", "edit", "--topic", "codec"]).ok();
    world.write("shared.txt", "codec amended\n");
    world.git(&["add", "shared.txt"]);
    world.git(&["commit", "--quiet", "--amend", "--no-edit"]);
    let run = world.stack(&["restack", "continue"]);
    assert_eq!(run.code, 3, "{}", run.stderr);
    (world, original)
}

#[test]
fn a_conflict_stops_and_can_be_skipped() {
    let (world, _) = conflicting();
    assert!(rebasing(&world));
    world.stack(&["restack", "skip"]).ok();
    assert!(!rebasing(&world));
    assert_eq!(subjects(&world), ["add the index", "add the codec"]);
}

#[test]
fn a_conflict_can_be_resolved_and_continued() {
    let (world, _) = conflicting();
    let unresolved = world.stack(&["restack", "continue"]);
    assert_eq!(unresolved.code, 2, "{}", unresolved.stderr);
    assert!(rebasing(&world));

    world.write("shared.txt", "resolved\n");
    world.git(&["add", "shared.txt"]);
    world.stack(&["restack", "continue"]).ok();
    assert!(!rebasing(&world));
    assert_eq!(subjects(&world), ALL);
    assert_eq!(world.git(&["show", "HEAD:shared.txt"]), "resolved");
}

#[test]
fn abort_puts_the_branch_back_exactly() {
    let (world, original) = conflicting();
    let branch = world.git(&["rev-parse", "--abbrev-ref", "HEAD"]);
    assert_eq!(branch, "HEAD", "the rebase has HEAD detached while stopped");
    world.stack(&["restack", "abort"]).ok();
    assert_eq!(head(&world), original);
    assert_ne!(world.git(&["rev-parse", "--abbrev-ref", "HEAD"]), "HEAD");
    assert!(!rebasing(&world));
    assert!(!world.work.join(".git/stack/restack.json").exists());
    assert_eq!(restack_status(&world), Value::Null);
}

#[test]
fn bad_requests_change_nothing() {
    let (world, _) = three_topics();
    let before = head(&world);

    world.write("codec.rs", "dirty\n");
    let dirty = world.stack(&["restack", "edit", "--topic", "codec"]);
    assert_eq!(dirty.code, 2, "{}", dirty.stderr);
    world.git(&["checkout", "--quiet", "--", "codec.rs"]);

    let none = world.stack(&["restack", "continue"]);
    assert_eq!(none.code, 2, "{}", none.stderr);
    let none = world.stack(&["restack", "abort"]);
    assert_eq!(none.code, 2, "{}", none.stderr);
    let unknown = world.stack(&["restack", "edit", "--topic", "nope"]);
    assert_eq!(unknown.code, 2, "{}", unknown.stderr);

    assert_eq!(head(&world), before);
    assert!(!rebasing(&world));
}

#[test]
fn upload_is_refused_while_a_restack_is_stopped() {
    let (world, _) = three_topics();
    world.stack(&["restack", "edit", "--topic", "codec"]).ok();
    let run = world.stack(&["upload"]);
    assert_eq!(run.code, 2, "{}", run.stderr);
    assert_eq!(world.remote_git(&["for-each-ref", "--format=%(refname)", "refs/heads"]), "refs/heads/main");
}

#[test]
fn a_stopped_restack_shows_in_status_and_resumes_in_a_fresh_process() {
    let (world, _) = three_topics();
    world.stack(&["restack", "edit", "--topic", "journal"]).ok();
    let stopped = restack_status(&world);
    assert!(!stopped.is_null(), "{stopped}");

    // Every command is a new process, so continuing proves the state is on disk.
    world.stack(&["restack", "continue"]).ok();
    assert_eq!(subjects(&world), ALL);
    assert_eq!(restack_status(&world), Value::Null);
}
