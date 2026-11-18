//! Revision pins and `stack diff`: every upload pins each moved head and its
//! base under refs/stack/<prefix>/<topic>/, so old revisions survive the
//! force-push and can be compared with the base's changes left out.

mod support;

use support::{World, history_rows, message};

/// The pin refs on the remote, as full names.
fn pins(world: &World) -> Vec<String> {
    let listed = world.remote_git(&["for-each-ref", "--format=%(refname)", "refs/stack/"]);
    listed.lines().map(str::to_owned).collect()
}

/// Lands a commit on the remote's main that the stack doesn't have, then
/// rebases the stack onto it.
fn move_trunk_and_rebase(world: &World) {
    let branch = world.git(&["symbolic-ref", "--short", "HEAD"]);
    world.git(&["checkout", "--quiet", "--detach", "origin/main"]);
    world.commit("trunk.rs", "someone else's change\n", "land the trunk change");
    world.git(&["push", "--quiet", "origin", "HEAD:refs/heads/main"]);
    world.git(&["checkout", "--quiet", &branch]);
    world.git(&["fetch", "--quiet", "origin"]);
    world.git(&["rebase", "--quiet", "origin/main"]);
}

#[test]
fn an_amend_after_a_rebase_keeps_the_old_revision_and_diff_shows_only_the_patch_change() {
    let world = World::new();
    world.commit("codec.rs", "codec v1\n", &message("add the codec", "Topic: codec"));
    world.stack(&["upload"]).ok();
    let first = world.remote_branch("alice/stack/codec").expect("first upload pushed the branch");

    move_trunk_and_rebase(&world);
    world.amend("codec.rs", "codec v2\n");
    world.stack(&["upload"]).ok();
    let second = world.remote_branch("alice/stack/codec").expect("the branch");
    assert_ne!(first, second);

    // The force-push left the first revision pinned and fetchable.
    assert_eq!(world.remote_git(&["rev-parse", "refs/stack/alice/codec/r1"]), first);
    assert_eq!(world.remote_git(&["rev-parse", "refs/stack/alice/codec/r2"]), second);
    let fresh = World::new();
    fresh.git(&["fetch", "--quiet", world.remote.to_str().expect("utf-8 path"), "refs/stack/alice/codec/r1"]);
    assert_eq!(fresh.git(&["rev-parse", "FETCH_HEAD"]), first);

    // The history row of the rebase names the command for its range.
    let rows = history_rows(&world.pr("alice/stack/codec"));
    assert!(rows.last().is_some_and(|row| row.contains("stack diff codec 1 2")), "{rows:#?}");

    for args in [&["diff", "codec"][..], &["diff", "codec", "1", "2"], &["diff", "codec", "r1", "r2"]] {
        let run = world.stack(args);
        run.ok();
        assert!(run.stdout.contains("codec v2"), "the patch change shows: {}", run.stdout);
        assert!(!run.stdout.contains("someone else's change"), "the base change stays out: {}", run.stdout);
    }
}

#[test]
fn every_upload_that_moves_a_topic_pins_it_and_an_unmoved_one_pins_nothing_new() {
    let world = World::new();
    world.commit("codec.rs", "codec\n", &message("add the codec", "Topic: codec"));
    world.commit("journal.rs", "journal\n", &message("compress the journal", "Topic: journal\nRelative: codec"));
    world.stack(&["upload"]).ok();
    let after_first = pins(&world);
    assert!(after_first.contains(&"refs/stack/alice/codec/r1".to_owned()), "{after_first:#?}");
    assert!(after_first.contains(&"refs/stack/alice/codec/r1-base".to_owned()), "{after_first:#?}");
    assert!(after_first.contains(&"refs/stack/alice/journal/r1".to_owned()), "{after_first:#?}");

    world.stack(&["upload"]).ok();
    assert_eq!(pins(&world), after_first);

    world.amend("journal.rs", "journal v2\n");
    world.stack(&["upload"]).ok();
    let after_amend = pins(&world);
    assert!(after_amend.contains(&"refs/stack/alice/journal/r2".to_owned()), "{after_amend:#?}");
    assert!(!after_amend.contains(&"refs/stack/alice/codec/r2".to_owned()), "{after_amend:#?}");
}

#[test]
fn pins_can_be_turned_off_or_pruned() {
    let off = World::new();
    off.commit("codec.rs", "codec\n", &message("add the codec", "Topic: codec"));
    off.config("stack.toml", "pin_revisions = false\n");
    off.stack(&["upload"]).ok();
    assert!(pins(&off).is_empty(), "{:#?}", pins(&off));

    let pruned = World::new();
    pruned.commit("codec.rs", "codec\n", &message("add the codec", "Topic: codec"));
    pruned.config("stack.toml", "keep_revisions = 2\n");
    for text in ["codec v2\n", "codec v3\n"] {
        pruned.stack(&["upload"]).ok();
        pruned.amend("codec.rs", text);
    }
    pruned.stack(&["upload"]).ok();
    let kept = pins(&pruned);
    assert!(!kept.iter().any(|name| name.starts_with("refs/stack/alice/codec/r1")), "{kept:#?}");
    for name in ["r2", "r2-base", "r3", "r3-base"] {
        assert!(kept.contains(&format!("refs/stack/alice/codec/{name}")), "{kept:#?}");
    }
}

#[test]
fn diff_with_nothing_to_compare_says_what_to_do() {
    let world = World::new();
    world.commit("codec.rs", "codec\n", &message("add the codec", "Topic: codec"));
    assert_eq!(world.stack(&["diff", "codec"]).code, 2);
    world.stack(&["upload"]).ok();
    assert_eq!(world.stack(&["diff", "codec"]).code, 2);
    assert_eq!(world.stack(&["diff", "codec", "1", "7"]).code, 2);
    assert_eq!(world.stack(&["diff", "codec", "latest"]).code, 2);
}
