//! `stack upload` against a local bare remote and the fake gh.

mod support;

use serde_json::Value;
use support::{World, history_rows, message};

/// A branch with topic `codec`, topic `journal` relative to it, and an
/// untagged commit on top.
fn two_topics() -> World {
    let world = World::new();
    world.commit("codec.rs", "codec v1\n", &message("add the codec", "Topic: codec"));
    world.commit(
        "journal.rs",
        "journal v1\n",
        &message("compress the journal", "Topic: journal\nRelative: codec\nReviewers: bob"),
    );
    world.commit("scratch.txt", "local only\n", "wip: local notes");
    world
}

#[test]
fn upload_opens_one_pr_per_topic_chained_by_base() {
    let world = two_topics();
    world.stack(&["upload"]).ok();

    let prs = world.prs();
    assert_eq!(prs.len(), 2, "{prs:#?}");
    let codec = world.pr("alice/stack/codec");
    let journal = world.pr("alice/stack/journal");
    assert_eq!(codec["base"], "main");
    assert_eq!(journal["base"], "alice/stack/codec");
    assert_eq!(codec["title"], "add the codec");
    assert_eq!(journal["title"], "compress the journal");
    assert_eq!(journal["reviewers"], serde_json::json!(["bob"]));

    // The journal branch sits on the codec branch, and the untagged commit
    // is pushed nowhere.
    let codec_head = world.remote_branch("alice/stack/codec").expect("codec branch pushed");
    let journal_head = world.remote_branch("alice/stack/journal").expect("journal branch pushed");
    assert_eq!(world.remote_git(&["rev-parse", &format!("{journal_head}^")]), codec_head);
    let pushed = world.remote_git(&["log", "--format=%s", "--all"]);
    assert!(!pushed.contains("wip: local notes"), "{pushed}");
}

#[test]
fn the_description_comes_from_the_template_and_the_commit() {
    let world = two_topics();
    world.stack(&["upload"]).ok();

    let body = world.pr("alice/stack/codec")["body"].as_str().unwrap_or_default().to_owned();
    assert!(body.contains("## Summary"), "{body}");
    assert!(body.contains("What add the codec changes."), "{body}");
    assert!(body.contains("## Testing"), "{body}");
    assert!(!body.contains("Topic:"), "trailers stay out of the description: {body}");
}

#[test]
fn each_pr_gets_one_history_comment_with_a_first_row() {
    let world = two_topics();
    world.stack(&["upload"]).ok();

    for branch in ["alice/stack/codec", "alice/stack/journal"] {
        let pr = world.pr(branch);
        let rows = history_rows(&pr);
        assert_eq!(rows.len(), 1, "{branch}: {rows:#?}");
        let head = world.remote_branch(branch).expect("branch pushed");
        assert!(rows[0].contains(&head), "the row links the head commit: {}", rows[0]);
    }
}

#[test]
fn uploading_after_an_amend_updates_the_same_prs_and_adds_a_row() {
    let world = World::new();
    world.commit("codec.rs", "codec v1\n", &message("add the codec", "Topic: codec"));
    world.commit(
        "journal.rs",
        "journal v1\n",
        &message("compress the journal", "Topic: journal\nRelative: codec"),
    );
    world.stack(&["upload"]).ok();
    let first_journal_head = world.remote_branch("alice/stack/journal");

    world.amend("journal.rs", "journal v2\n");
    world.stack(&["upload", "--note", "addressed review"]).ok();

    assert_eq!(world.prs().len(), 2, "no new PRs: {:#?}", world.prs());
    let journal = world.pr("alice/stack/journal");
    let rows = history_rows(&journal);
    assert_eq!(rows.len(), 2, "{rows:#?}");
    assert!(rows[1].contains("addressed review"), "{}", rows[1]);
    assert_ne!(world.remote_branch("alice/stack/journal"), first_journal_head);
    let journal_head = world.remote_branch("alice/stack/journal").expect("journal branch");
    assert_eq!(world.remote_git(&["show", &format!("{journal_head}:journal.rs")]), "journal v2");

    // The codec topic didn't change: no new row, same head.
    assert_eq!(history_rows(&world.pr("alice/stack/codec")).len(), 1);
}

#[test]
fn uploading_again_without_changes_adds_no_row_and_pushes_nothing() {
    let world = two_topics();
    world.stack(&["upload"]).ok();
    let heads = (world.remote_branch("alice/stack/codec"), world.remote_branch("alice/stack/journal"));

    world.stack(&["upload"]).ok();

    assert_eq!(world.prs().len(), 2);
    assert_eq!(history_rows(&world.pr("alice/stack/codec")).len(), 1);
    assert_eq!(history_rows(&world.pr("alice/stack/journal")).len(), 1);
    assert_eq!(
        (world.remote_branch("alice/stack/codec"), world.remote_branch("alice/stack/journal")),
        heads
    );
}

#[test]
fn a_dry_run_changes_nothing() {
    let world = two_topics();
    let run = world.stack(&["upload", "--dry-run"]);
    run.ok();

    assert!(world.prs().is_empty(), "{:#?}", world.prs());
    assert_eq!(world.remote_branch("alice/stack/codec"), None);
    assert!(run.stdout.contains("codec") && run.stdout.contains("journal"), "{}", run.stdout);
}

#[test]
fn a_topic_spread_over_several_commits_is_one_pr() {
    let world = World::new();
    world.commit("codec.rs", "codec v1\n", &message("add the codec", "Topic: codec"));
    world.commit("journal.rs", "journal v1\n", &message("compress the journal", "Topic: journal\nRelative: codec"));
    world.commit("bench.rs", "bench\n", &message("bench the codec", "Topic: codec"));
    world.stack(&["upload"]).ok();

    assert_eq!(world.prs().len(), 2);
    let codec_head = world.remote_branch("alice/stack/codec").expect("codec branch");
    let subjects = world.remote_git(&["log", "--format=%s", &format!("main..{codec_head}")]);
    assert_eq!(subjects, "bench the codec\nadd the codec");
    let journal_head = world.remote_branch("alice/stack/journal").expect("journal branch");
    assert_eq!(world.remote_git(&["rev-parse", &format!("{journal_head}^")]), codec_head);
    assert!(
        world.pr("alice/stack/codec")["body"].as_str().unwrap_or_default().contains("## Commits"),
        "a topic with two commits lists them"
    );
}

#[test]
fn upload_of_one_topic_takes_the_topics_it_builds_on() {
    let world = World::new();
    world.commit("codec.rs", "codec\n", &message("add the codec", "Topic: codec"));
    world.commit("journal.rs", "journal\n", &message("compress the journal", "Topic: journal\nRelative: codec"));
    world.commit("docs.md", "docs\n", &message("document storage", "Topic: docs"));
    world.stack(&["upload", "--topic", "journal"]).ok();

    let heads: Vec<String> = world.prs().iter().filter_map(|pr| pr["head"].as_str().map(str::to_owned)).collect();
    assert_eq!(heads.len(), 2, "{heads:?}");
    assert!(heads.contains(&"alice/stack/codec".to_owned()));
    assert!(heads.contains(&"alice/stack/journal".to_owned()));
}

#[test]
fn a_closed_pr_stops_the_upload_before_anything_is_pushed() {
    let world = two_topics();
    world.stack(&["upload"]).ok();
    let before = world.remote_branch("alice/stack/journal");
    world.edit_state(|state| {
        if let Some(prs) = state["prs"].as_array_mut() {
            for pr in prs.iter_mut().filter(|pr| pr["head"] == "alice/stack/codec") {
                pr["state"] = Value::from("CLOSED");
            }
        }
    });
    world.git(&["checkout", "--quiet", "HEAD~1"]);
    world.amend("journal.rs", "journal v2\n");

    let run = world.stack(&["upload"]);
    assert_eq!(run.code, 4, "stdout:\n{}\nstderr:\n{}", run.stdout, run.stderr);
    assert_eq!(world.remote_branch("alice/stack/journal"), before);
}

#[test]
fn bad_trailers_stop_the_upload_with_a_usage_error() {
    let world = World::new();
    world.commit("a.rs", "a\n", &message("add a", "Topic: a\nRelative: missing"));
    let run = world.stack(&["upload"]);
    assert_eq!(run.code, 2, "stderr:\n{}", run.stderr);
    assert!(world.prs().is_empty());
    assert_eq!(world.remote_branch("alice/stack/a"), None);
}

#[test]
fn a_chain_is_linked_as_a_native_stack_when_gh_has_the_extension() {
    let world = two_topics();
    world.edit_state(|state| state["native_stack"] = Value::from("linked"));
    world.stack(&["upload"]).ok();

    let codec = world.pr("alice/stack/codec")["url"].clone();
    let journal = world.pr("alice/stack/journal")["url"].clone();
    let stacks = world.state()["stacks"].clone();
    assert_eq!(stacks, serde_json::json!([[codec, journal]]));
}

#[test]
fn a_refused_native_stack_still_leaves_the_chained_prs() {
    let world = two_topics();
    world.edit_state(|state| state["native_stack"] = Value::from("refused"));
    world.stack(&["upload"]).ok();

    assert_eq!(world.prs().len(), 2);
    assert_eq!(world.pr("alice/stack/journal")["base"], "alice/stack/codec");
}

#[test]
fn without_the_extension_or_with_the_opt_out_no_native_link_is_tried() {
    let world = two_topics();
    world.stack(&["upload"]).ok();
    assert!(!world.calls().iter().any(|call| call.starts_with("stack link")), "{:#?}", world.calls());

    let world = two_topics();
    world.edit_state(|state| state["native_stack"] = Value::from("linked"));
    world.stack(&["upload", "--no-native-stack"]).ok();
    assert!(!world.calls().iter().any(|call| call.starts_with("stack link")), "{:#?}", world.calls());
}

#[test]
fn a_description_edited_on_github_is_kept_unless_overwritten() {
    let world = two_topics();
    world.stack(&["upload"]).ok();
    world.edit_state(|state| {
        if let Some(prs) = state["prs"].as_array_mut() {
            for pr in prs.iter_mut().filter(|pr| pr["head"] == "alice/stack/codec") {
                pr["body"] = Value::from("written by hand on GitHub");
            }
        }
    });
    world.git(&["checkout", "--quiet", "HEAD~2"]);
    world.amend("codec.rs", "codec v2\n");

    world.stack(&["upload"]).ok();
    assert_eq!(world.pr("alice/stack/codec")["body"], "written by hand on GitHub");

    world.stack(&["upload", "--overwrite-body"]).ok();
    let body = world.pr("alice/stack/codec")["body"].as_str().unwrap_or_default().to_owned();
    assert!(body.contains("## Summary"), "{body}");
}

#[test]
fn json_output_names_each_topics_pr() {
    let world = two_topics();
    let run = world.stack(&["upload", "--json"]);
    run.ok();
    let printed: Value = serde_json::from_str(&run.stdout).expect("upload --json prints JSON");
    let topics: Vec<&str> = printed
        .as_array()
        .expect("an array")
        .iter()
        .filter_map(|item| item["topic"].as_str())
        .collect();
    assert_eq!(topics, ["codec", "journal"]);
    for item in printed.as_array().into_iter().flatten() {
        assert!(item["number"].is_u64() && item["url"].is_string(), "{item}");
    }
}

#[test]
fn the_configured_template_shapes_the_description() {
    let world = World::new();
    world.config("pr.md", "## Summary\n<!-- required -->\n\n## Rollout\n");
    world.config("stack.toml", "template = \"pr.md\"\n");
    world.commit(
        "a.rs",
        "a\n",
        "add a\n\nIt adds a.\n\n## Rollout\nBehind a flag.\n\nTopic: a\n",
    );
    world.stack(&["upload"]).ok();

    let body = world.pr("alice/stack/a")["body"].as_str().unwrap_or_default().to_owned();
    assert!(body.contains("## Summary\n\nIt adds a."), "{body}");
    assert!(body.contains("## Rollout\n\nBehind a flag."), "{body}");
}

#[test]
fn every_description_lists_the_whole_stack() {
    let world = two_topics();
    world.stack(&["upload"]).ok();
    let codec = world.pr("alice/stack/codec");
    let journal = world.pr("alice/stack/journal");
    let body = |pr: &Value| pr["body"].as_str().unwrap_or_default().to_owned();
    assert!(body(&codec).contains(&format!("#{}", journal["number"])), "{}", body(&codec));
    assert!(body(&journal).contains(&format!("#{}", codec["number"])), "{}", body(&journal));

    // Written once: another upload leaves the descriptions as they are.
    world.stack(&["upload"]).ok();
    assert_eq!(body(&world.pr("alice/stack/codec")), body(&codec));
}

#[test]
fn uploading_one_topic_keeps_the_whole_stack_in_its_description() {
    let world = World::new();
    world.commit("codec.rs", "codec\n", &message("add the codec", "Topic: codec"));
    world.commit("journal.rs", "journal\n", &message("compress the journal", "Topic: journal\nRelative: codec"));
    world.commit("index.rs", "index\n", &message("add the index", "Topic: index\nRelative: journal"));
    world.stack(&["upload"]).ok();
    let number = |branch: &str| format!("#{}", world.pr(branch)["number"]);
    let (journal, index) = (number("alice/stack/journal"), number("alice/stack/index"));

    // codec builds on nothing, so this run touches only codec's pull request.
    world.stack(&["upload", "--topic", "codec"]).ok();
    let body = world.pr("alice/stack/codec")["body"].as_str().unwrap_or_default().to_owned();
    assert!(body.contains(&journal) && body.contains(&index), "{body}");
}
