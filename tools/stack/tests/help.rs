//! The help and the guide are the manual: every command documents itself.

mod support;

use serde_json::Value;
use support::World;

/// Every command, as `stack commands --json` lists it from the CLI
/// definition: `["restack", "edit"]` and so on.
fn commands(world: &World) -> Vec<Vec<String>> {
    let run = world.stack(&["commands", "--json"]);
    run.ok();
    let printed: Value = serde_json::from_str(&run.stdout).expect("commands --json prints JSON");
    printed["commands"]
        .as_array()
        .expect("commands")
        .iter()
        .map(|command| {
            let path = command["path"].as_str().expect("path");
            path.split(' ').map(str::to_owned).collect()
        })
        .collect()
}

#[test]
fn help_lists_every_command() {
    let world = World::new();
    let all = commands(&world);
    for expected in ["check", "upload", "status", "guide", "restack", "diff", "config", "commands"] {
        assert!(all.iter().any(|path| path[0] == expected), "commands --json lacks {expected}: {all:?}");
    }
    let run = world.stack(&["--help"]);
    run.ok();
    for path in &all {
        assert!(run.stdout.contains(&path[0]), "--help doesn't list {}:\n{}", path[0], run.stdout);
    }
    assert!(run.stdout.contains("Topic:") && run.stdout.contains("Relative:"), "{}", run.stdout);
}

#[test]
fn every_command_has_its_own_help_listing_its_flags() {
    let world = World::new();
    let run = world.stack(&["commands", "--json"]);
    let printed: Value = serde_json::from_str(&run.ok().stdout).expect("JSON");
    for command in printed["commands"].as_array().expect("commands") {
        let path: Vec<&str> = command["path"].as_str().expect("path").split(' ').collect();
        let mut args = path.clone();
        args.push("--help");
        let help = world.stack(&args);
        help.ok();
        assert!(help.stdout.contains("Usage:"), "{path:?} --help:\n{}", help.stdout);
        for arg in command["args"].as_array().into_iter().flatten() {
            let name = arg["name"].as_str().expect("name");
            let shown = if arg["positional"] == true {
                arg["value"].as_str().unwrap_or(name).to_owned()
            } else {
                name.to_owned()
            };
            assert!(
                help.stdout.to_lowercase().contains(&shown.to_lowercase()),
                "{path:?} --help lacks {shown}:\n{}",
                help.stdout
            );
        }
    }
}

#[test]
fn the_guide_explains_trailers_and_descriptions() {
    let world = World::new();
    let run = world.stack(&["guide"]);
    run.ok();
    for expected in ["Topic:", "Relative:", "## Summary", "stack upload"] {
        assert!(run.stdout.contains(expected), "guide lacks {expected}:\n{}", run.stdout);
    }
}

#[test]
fn guide_template_shows_the_template_in_use() {
    let world = World::new();
    let run = world.stack(&["guide", "--template"]);
    run.ok();
    assert!(run.stdout.contains("## Summary"), "{}", run.stdout);

    // The repository's own GitHub template is GitHub's web default only.
    world.commit(".github/pull_request_template.md", "## Ignored\n", "add a template");
    world.config("pr.md", "## Changes\n");
    world.config("stack.toml", "template = \"pr.md\"\n");
    let run = world.stack(&["guide", "--template"]);
    run.ok();
    assert!(run.stdout.contains("## Changes"), "{}", run.stdout);
    assert!(!run.stdout.contains("## Ignored"), "{}", run.stdout);
}

#[test]
fn an_unknown_command_is_a_usage_error() {
    let world = World::new();
    assert_eq!(world.stack(&["frobnicate"]).code, 2);
}
