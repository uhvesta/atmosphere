//! Tests for `stack import`, interleaving local commits between imported
//! layers, safe restacking without force-pushing foreign branches, and
//! re-importing after remote changes.

mod support;

use serde_json::{Value, json};
use support::{World, message};

/// Sets up a 3-PR remote stack authored by `bob`:
///   main <- bob/stack/layer1 (#1) <- bob/stack/layer2 (#2) <- bob/stack/layer3 (#3)
/// Returns the remote SHAs `(sha1, sha2, sha3)` and leaves the local clone on `main`.
fn seed_three_pr_remote_stack(world: &World) -> (String, String, String) {
    let main_sha = world.git(&["rev-parse", "main"]);

    world.git(&["checkout", "--quiet", "-b", "bob/stack/layer1", &main_sha]);
    let sha1 = world.commit("l1.txt", "layer 1 v1\n", "bob: add layer 1");
    world.git(&["push", "--quiet", "origin", "HEAD:refs/heads/bob/stack/layer1"]);

    world.git(&["checkout", "--quiet", "-b", "bob/stack/layer2", &sha1]);
    let sha2 = world.commit("l2.txt", "layer 2 v1\n", "bob: add layer 2");
    world.git(&["push", "--quiet", "origin", "HEAD:refs/heads/bob/stack/layer2"]);

    world.git(&["checkout", "--quiet", "-b", "bob/stack/layer3", &sha2]);
    let sha3 = world.commit("l3.txt", "layer 3 v1\n", "bob: add layer 3");
    world.git(&["push", "--quiet", "origin", "HEAD:refs/heads/bob/stack/layer3"]);

    // Return local checkout to clean `main` and remove local bob branches so
    // `stack import` must fetch them from the remote.
    world.git(&["checkout", "--quiet", "main"]);
    for branch in ["bob/stack/layer1", "bob/stack/layer2", "bob/stack/layer3"] {
        world.git(&["branch", "-D", branch]);
    }

    world.edit_state(|state| {
        state["next_number"] = json!(4);
        state["prs"] = json!([
            {
                "number": 1,
                "url": "https://github.com/o/r/pull/1",
                "head": "bob/stack/layer1",
                "base": "main",
                "title": "bob: add layer 1",
                "body": "layer 1",
                "state": "OPEN",
                "draft": false,
                "author": "bob",
                "reviewers": [],
                "labels": [],
                "comments": []
            },
            {
                "number": 2,
                "url": "https://github.com/o/r/pull/2",
                "head": "bob/stack/layer2",
                "base": "bob/stack/layer1",
                "title": "bob: add layer 2",
                "body": "layer 2",
                "state": "OPEN",
                "draft": false,
                "author": "bob",
                "reviewers": [],
                "labels": [],
                "comments": []
            },
            {
                "number": 3,
                "url": "https://github.com/o/r/pull/3",
                "head": "bob/stack/layer3",
                "base": "bob/stack/layer2",
                "title": "bob: add layer 3",
                "body": "layer 3",
                "state": "OPEN",
                "draft": false,
                "author": "bob",
                "reviewers": [],
                "labels": [],
                "comments": []
            }
        ]);
    });

    (sha1, sha2, sha3)
}

#[test]
fn import_three_pr_remote_stack_and_run_status_diff_and_restack() {
    let world = World::new();
    seed_three_pr_remote_stack(&world);

    // Import by PR URL of the middle PR (#2): discovers the full 3-PR chain.
    let imported = world.stack(&[
        "import",
        "https://github.com/o/r/pull/2",
        "--json",
    ]);
    imported.ok();
    let imp: Value = serde_json::from_str(&imported.stdout).expect("import JSON");
    assert_eq!(imp["source"], "base_chain");
    let layers = imp["layers"].as_array().expect("layers");
    assert_eq!(layers.len(), 3, "{imp:#}");
    assert_eq!(layers[0]["topic"], "layer1");
    assert_eq!(layers[1]["topic"], "layer2");
    assert_eq!(layers[1]["relative"], "layer1");
    assert_eq!(layers[2]["topic"], "layer3");
    assert_eq!(layers[2]["relative"], "layer2");
    assert!(layers.iter().all(|l| l["foreign"] == true));

    // `status --json` understands the imported stack layer by layer.
    let st = world.stack(&["status", "--json"]);
    st.ok();
    let status: Value = serde_json::from_str(&st.stdout).expect("status JSON");
    let topics = status["topics"].as_array().expect("topics");
    assert_eq!(topics.len(), 3, "{status:#}");
    assert_eq!(topics[0]["number"], 1);
    assert_eq!(topics[1]["number"], 2);
    assert_eq!(topics[2]["number"], 3);
    assert_eq!(topics[0]["foreign"], true);

    // `diff` works on an imported layer even without remote pins.
    let diff = world.stack(&["diff", "layer2"]);
    diff.ok();
    assert!(diff.stdout.contains("layer 2 v1"), "{}", diff.stdout);

    // `restack edit` + `continue` works on the imported stack.
    world.stack(&["restack", "edit", "--topic", "layer2"]).ok();
    world.stack(&["restack", "continue"]).ok();
}

#[test]
fn import_from_native_gh_stack_works() {
    let world = World::new();
    seed_three_pr_remote_stack(&world);
    world.edit_state(|state| {
        state["native_stack"] = json!("linked");
        state["native_stacks"] = json!([{
            "number": 7,
            "base": "main",
            "prs": [1, 2, 3],
        }]);
    });

    let imported = world.stack(&["import", "stack:7", "--json"]);
    imported.ok();
    let imp: Value = serde_json::from_str(&imported.stdout).expect("import JSON");
    assert_eq!(imp["source"], "native");
    assert_eq!(imp["github_stack_number"], 7);
    assert_eq!(imp["layers"].as_array().map(Vec::len), Some(3));
}

#[test]
fn interleave_two_local_commits_between_imported_layers_updates_bases_without_foreign_force_push() {
    let world = World::new();
    let (sha1, sha2, sha3) = seed_three_pr_remote_stack(&world);

    world.stack(&["import", "2"]).ok();

    // Interleave 2 local commits between the imported layers:
    //   layer1 -> local-a -> layer2 -> local-b -> layer3
    // Use `stack restack edit` to insert `local-a` right after `layer1` and
    // update `layer2`'s Relative to `local-a`, and `local-b` after `layer2`
    // with `layer3`'s Relative updated to `local-b`.
    world.stack(&["restack", "edit", "--topic", "layer1"]).ok();
    world.commit(
        "local_a.txt",
        "local a\n",
        &message("alice: add local a", "Topic: local-a\nRelative: layer1"),
    );
    world.stack(&["restack", "continue", "--step"]).ok();
    // Now stopped right after `layer2`: reword layer2 to depend on local-a,
    // and add `local-b` on top of `layer2`.
    world.git(&[
        "commit",
        "--amend",
        "--quiet",
        "-m",
        "bob: add layer 2\n\nTopic: layer2\nRelative: local-a\n",
    ]);
    world.commit(
        "local_b.txt",
        "local b\n",
        &message("alice: add local b", "Topic: local-b\nRelative: layer2"),
    );
    world.stack(&["restack", "continue"]).ok();
    // Reword HEAD (`layer3`) to depend on `local-b`.
    world.git(&[
        "commit",
        "--amend",
        "--quiet",
        "-m",
        "bob: add layer 3\n\nTopic: layer3\nRelative: local-b\n",
    ]);

    // `stack check` validates the interleaved 5-layer stack.
    let check = world.stack(&["check", "--json"]);
    check.ok();
    let plan: Value = serde_json::from_str(&check.stdout).expect("check JSON");
    let names: Vec<&str> = plan["topics"]
        .as_array()
        .expect("topics")
        .iter()
        .filter_map(|t| t["topic"].as_str())
        .collect();
    assert_eq!(names, ["layer1", "local-a", "layer2", "local-b", "layer3"]);

    // Upload WITHOUT `--force-foreign`:
    // - local-a and local-b are pushed and opened as PRs with the right bases;
    // - foreign PRs #2 and #3 have their bases updated on GitHub;
    // - foreign branches on the remote are NOT force-pushed!
    let up = world.stack(&["upload"]);
    up.ok();

    assert_eq!(
        world.remote_branch("bob/stack/layer1").as_deref(),
        Some(sha1.as_str()),
        "foreign branch layer1 must not be force-pushed"
    );
    assert_eq!(
        world.remote_branch("bob/stack/layer2").as_deref(),
        Some(sha2.as_str()),
        "foreign branch layer2 must not be force-pushed"
    );
    assert_eq!(
        world.remote_branch("bob/stack/layer3").as_deref(),
        Some(sha3.as_str()),
        "foreign branch layer3 must not be force-pushed"
    );

    assert!(world.remote_branch("alice/stack/local-a").is_some());
    assert!(world.remote_branch("alice/stack/local-b").is_some());
    assert_eq!(world.pr("alice/stack/local-a")["base"], "bob/stack/layer1");
    assert_eq!(world.pr("bob/stack/layer2")["base"], "alice/stack/local-a");
    assert_eq!(world.pr("alice/stack/local-b")["base"], "bob/stack/layer2");
    assert_eq!(world.pr("bob/stack/layer3")["base"], "alice/stack/local-b");
}

#[test]
fn reimport_after_remote_change_updates_in_place_and_keeps_local_commits() {
    let world = World::new();
    let (_sha1, sha2, _sha3) = seed_three_pr_remote_stack(&world);

    world.stack(&["import", "1"]).ok();
    // Add a local topic on top of the imported stack.
    world.commit(
        "alice_top.txt",
        "alice top\n",
        &message("alice: top layer", "Topic: alice-top\nRelative: layer3"),
    );

    // Simulate Bob updating `bob/stack/layer2` and `bob/stack/layer3` on the remote.
    let work_branch = world.git(&["symbolic-ref", "--short", "HEAD"]);
    world.git(&["checkout", "--quiet", "--detach", &sha2]);
    let new_sha2 = world.commit("l2_extra.txt", "layer 2 v2\n", "bob: update layer 2");
    world.git(&["push", "--quiet", "--force", "origin", &format!("{new_sha2}:refs/heads/bob/stack/layer2")]);
    let new_sha3 = world.commit("l3.txt", "layer 3 v2\n", "bob: update layer 3");
    world.git(&["push", "--quiet", "--force", "origin", &format!("{new_sha3}:refs/heads/bob/stack/layer3")]);
    world.git(&["checkout", "--quiet", &work_branch]);

    // Re-importing updates the local branch in place with Bob's new commits
    // and preserves Alice's local topic `alice-top`.
    world.stack(&["import", "1"]).ok();
    assert_eq!(world.git(&["symbolic-ref", "--short", "HEAD"]), work_branch);

    let log = world.git(&["log", "--format=%s", "origin/main..HEAD"]);
    assert!(log.contains("bob: update layer 2"), "{log}");
    assert!(log.contains("bob: update layer 3"), "{log}");
    assert!(log.contains("alice: top layer"), "{log}");
}
