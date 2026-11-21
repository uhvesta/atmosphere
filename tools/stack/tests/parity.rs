//! Tests for `gh stack` verb parity, native-mode zero-comment behavior,
//! and repush summary diff-table formatting.

mod support;

use serde_json::{Value, json};
use support::{World, history_rows, message};

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

#[test]
fn init_add_and_navigation_verbs_match_gh_stack() {
    let world = World::new();
    world.stack(&["init", "--base", "main", "feat-a", "feat-b"]).ok();
    assert_eq!(world.git(&["symbolic-ref", "--short", "HEAD"]), "feat-b");

    world.stack(&["add", "feat-c"]).ok();
    assert_eq!(world.git(&["symbolic-ref", "--short", "HEAD"]), "feat-c");

    world.stack(&["down"]).ok();
    assert_eq!(world.git(&["symbolic-ref", "--short", "HEAD"]), "feat-b");

    world.stack(&["bottom"]).ok();
    assert_eq!(world.git(&["symbolic-ref", "--short", "HEAD"]), "feat-a");

    world.stack(&["up"]).ok();
    assert_eq!(world.git(&["symbolic-ref", "--short", "HEAD"]), "feat-b");

    world.stack(&["top"]).ok();
    assert_eq!(world.git(&["symbolic-ref", "--short", "HEAD"]), "feat-c");

    world.stack(&["trunk"]).ok();
    assert_eq!(world.git(&["symbolic-ref", "--short", "HEAD"]), "main");
}

#[test]
fn view_submit_push_sync_rebase_and_link_work_end_to_end() {
    let world = two_topics();

    // `view --json` mirrors `status --json`.
    let view = world.stack(&["view", "--json", "--offline"]);
    view.ok();
    let parsed: Value = serde_json::from_str(&view.stdout).expect("view --json");
    assert_eq!(parsed["topics"].as_array().map(Vec::len), Some(2));

    // `push` pushes branches without creating PRs.
    world.stack(&["push"]).ok();
    assert!(world.remote_branch("alice/stack/codec").is_some());
    assert!(world.remote_branch("alice/stack/journal").is_some());
    assert!(world.prs().is_empty());

    // `submit --auto` opens draft PRs non-interactively.
    world.stack(&["submit", "--auto"]).ok();
    assert_eq!(world.prs().len(), 2);
    assert_eq!(world.pr("alice/stack/codec")["draft"], true);
    assert_eq!(world.pr("alice/stack/journal")["draft"], true);

    // `submit --open` marks existing draft PRs ready for review.
    world.stack(&["submit", "--open"]).ok();
    assert_eq!(world.pr("alice/stack/codec")["draft"], false);
    assert_eq!(world.pr("alice/stack/journal")["draft"], false);

    // `link` chains and links PRs when native stacks are enabled.
    world.edit_state(|state| state["native_stack"] = Value::from("linked"));
    let pr1 = world.pr("alice/stack/codec")["number"].to_string();
    let pr2 = world.pr("alice/stack/journal")["number"].to_string();
    world.stack(&["link", "--base", "main", &pr1, &pr2]).ok();
    assert!(!world.state()["stacks"].as_array().unwrap().is_empty());

    // `rebase` and `sync` rebase cleanly onto an updated trunk.
    let current_head = world.git(&["rev-parse", "HEAD"]);
    world.git(&["checkout", "--quiet", "main"]);
    world.commit("trunk.txt", "trunk update\n", "trunk update");
    world.git(&["push", "--quiet", "origin", "main"]);
    world.git(&["checkout", "--quiet", "-B", "work", &current_head]);

    world.stack(&["rebase"]).ok();
    world.stack(&["sync"]).ok();
    assert_eq!(world.prs().len(), 2);
}

#[test]
fn native_stack_mode_writes_zero_comments_and_leaves_existing_comments_alone() {
    let world = two_topics();
    world.edit_state(|state| state["native_stack"] = Value::from("linked"));

    world.stack(&["upload"]).ok();

    let comment_calls: Vec<String> = world
        .calls()
        .into_iter()
        .filter(|call| call.contains("/comments"))
        .collect();
    assert!(
        comment_calls.is_empty(),
        "native stack mode must make zero comment API calls, saw: {comment_calls:#?}"
    );
    for branch in ["alice/stack/codec", "alice/stack/journal"] {
        assert!(
            history_rows(&world.pr(branch)).is_empty(),
            "no history comment should be written in native mode"
        );
    }

    // Seed an existing history comment on PR #1 and re-upload in native mode:
    // the existing comment must be left alone, never deleted or edited.
    world.edit_state(|state| {
        state["calls"] = json!([]);
        if let Some(prs) = state["prs"].as_array_mut() {
            prs[0]["comments"] = json!([{
                "id": 99,
                "body": "<!-- stack:history v1 -->\nexisting comment",
                "login": "alice",
            }]);
        }
    });
    world.amend("journal.rs", "journal v2\n");
    world.stack(&["upload"]).ok();

    let comment_calls_after: Vec<String> = world
        .calls()
        .into_iter()
        .filter(|call| call.contains("/comments"))
        .collect();
    assert!(
        comment_calls_after.is_empty(),
        "native mode must not touch comments on re-upload: {comment_calls_after:#?}"
    );
    assert_eq!(
        world.pr("alice/stack/codec")["comments"][0]["body"],
        "<!-- stack:history v1 -->\nexisting comment"
    );

    // With `--no-native-stack`, fallback comment mode writes comments as before.
    world.stack(&["upload", "--no-native-stack"]).ok();
    assert!(!history_rows(&world.pr("alice/stack/journal")).is_empty());
}

#[test]
fn repush_in_comment_mode_posts_summary_table_with_clickable_compare_links() {
    let world = two_topics();
    // Default fake_gh has native_stack = "absent" -> comment/table fallback mode.
    world.stack(&["upload"]).ok();
    let old_journal = world
        .remote_branch("alice/stack/journal")
        .expect("journal pushed");

    world.amend("journal.rs", "journal v2\n");
    world.stack(&["upload", "--note", "review fixes"]).ok();
    let new_journal = world
        .remote_branch("alice/stack/journal")
        .expect("journal repushed");
    assert_ne!(old_journal, new_journal);

    let journal_pr = world.pr("alice/stack/journal");
    let comment_body = journal_pr["comments"][0]["body"]
        .as_str()
        .expect("history comment exists");
    assert!(
        comment_body.contains("**Stack Update Summary**"),
        "expected Stack Update Summary table:\n{comment_body}"
    );
    let expected_compare = format!(
        "https://github.com/o/r/compare/{old_journal}...{new_journal}"
    );
    assert!(
        comment_body.contains(&expected_compare),
        "expected clickable compare link `{expected_compare}` in:\n{comment_body}"
    );
    assert_eq!(history_rows(&journal_pr).len(), 2);
}
