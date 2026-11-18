//! stack.toml: every setting is a compiled-in default or a key in
//! `$HUMATA2_HOME/config/stack.toml`; the binary and that folder are all
//! stack needs.

mod support;

use serde_json::Value;
use support::{World, message};

fn one_topic() -> World {
    let world = World::new();
    world.commit("codec.rs", "codec\n", &message("add the codec", "Topic: codec"));
    world
}

/// `stack config show --json`: each key's value and where it came from.
fn shown(world: &World) -> Vec<(String, Value, String)> {
    let run = world.stack(&["config", "show", "--json"]);
    run.ok();
    let printed: Value = serde_json::from_str(&run.stdout).expect("config show --json prints JSON");
    printed["settings"]
        .as_array()
        .expect("settings")
        .iter()
        .map(|row| {
            (
                row["key"].as_str().expect("key").to_owned(),
                row["value"].clone(),
                row["from"].as_str().expect("from").to_owned(),
            )
        })
        .collect()
}

fn setting(world: &World, key: &str) -> (Value, String) {
    shown(world)
        .into_iter()
        .find(|(name, _, _)| name == key)
        .map(|(_, value, from)| (value, from))
        .unwrap_or_else(|| panic!("config show has no {key}"))
}

#[test]
fn a_missing_file_is_fine_and_every_setting_is_a_default() {
    let world = one_topic();
    let settings = shown(&world);
    assert!(!settings.is_empty());
    assert!(settings.iter().all(|(_, _, from)| from == "default"), "{settings:#?}");
    world.stack(&["upload"]).ok();
    assert!(world.pr("alice/stack/codec").is_object());
}

#[test]
fn config_show_says_which_settings_came_from_the_file() {
    let world = one_topic();
    world.config("stack.toml", "prefix = \"foo\"\nnative_stack = false\n");
    assert_eq!(setting(&world, "prefix"), (Value::from("foo"), "stack.toml".to_owned()));
    assert_eq!(setting(&world, "native_stack"), (Value::from(false), "stack.toml".to_owned()));
    assert_eq!(setting(&world, "remote"), (Value::from("origin"), "default".to_owned()));
}

#[test]
fn unknown_secret_and_malformed_files_are_refused_before_anything_is_pushed() {
    for text in [
        "colour = \"red\"\n",
        "token = \"x\"\n",
        "gh_token = \"x\"\n",
        "prefix = \"ghp_0123456789\"\n",
        "prefix = \n",
        "draft = \"yes\"\n",
        "template = \"../elsewhere.md\"\n",
    ] {
        let world = one_topic();
        world.config("stack.toml", text);
        for args in [&["upload"][..], &["check"], &["config", "show"]] {
            let run = world.stack(args);
            assert_eq!(run.code, 2, "{text:?} {args:?}\nstderr:\n{}", run.stderr);
        }
        assert!(world.prs().is_empty(), "{text:?}: {:#?}", world.prs());
        assert_eq!(
            world.remote_git(&["for-each-ref", "--format=%(refname)", "refs/heads/"]),
            "refs/heads/main"
        );
    }
}

#[test]
fn a_missing_template_file_is_an_error_not_the_default() {
    let world = one_topic();
    world.config("stack.toml", "template = \"pr.md\"\n");
    assert_ne!(world.stack(&["check"]).code, 0);
}

#[test]
fn the_files_switches_change_what_upload_does() {
    let world = World::new();
    world.commit("codec.rs", "codec\n", &message("add the codec", "Topic: codec"));
    world.commit(
        "journal.rs",
        "journal\n",
        &message("compress the journal", "Topic: journal\nRelative: codec"),
    );
    world.edit_state(|state| state["native_stack"] = Value::from("linked"));
    world.config("stack.toml", "draft = true\nnative_stack = false\nhistory_comment = false\n");
    world.stack(&["upload"]).ok();

    let journal = world.pr("alice/stack/journal");
    assert_eq!(journal["draft"], true, "{journal:#}");
    assert!(world.state()["stacks"].as_array().is_none_or(Vec::is_empty));
    assert!(journal["comments"].as_array().is_none_or(Vec::is_empty), "{journal:#}");
}

#[test]
fn a_copied_binary_and_config_folder_behave_the_same() {
    let world = one_topic();
    world.config("stack.toml", "prefix = \"foo\"\ntemplate = \"pr.md\"\n");
    world.config("pr.md", "## Summary\n<!-- required -->\n");

    // A fresh machine: the binary and the config folder copied, nothing else.
    let fresh = tempfile::tempdir().expect("temp dir");
    let bin = fresh.path().join("stack");
    std::fs::copy(world.stack_bin(), &bin).expect("copy the binary");
    let config = fresh.path().join("home/config");
    std::fs::create_dir_all(&config).expect("mkdir");
    for name in ["stack.toml", "pr.md"] {
        std::fs::copy(world.humata_home.join("config").join(name), config.join(name)).expect("copy");
    }
    let home = fresh.path().join("home");

    let here = world.stack(&["config", "show", "--json"]);
    let there = world.stack_from(&bin, &home, &["config", "show", "--json"]);
    let settings = |run: &support::Run| -> Value {
        serde_json::from_str::<Value>(&run.ok().stdout).expect("JSON")["settings"].clone()
    };
    assert_eq!(settings(&here), settings(&there));

    world.stack_from(&bin, &home, &["upload"]).ok();
    assert!(world.pr("foo/stack/codec").is_object(), "{:#?}", world.prs());
}
