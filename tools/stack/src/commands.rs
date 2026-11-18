//! `stack commands` and `stack config show`: the command line and the
//! settings as data, for agents.

use clap::CommandFactory as _;
use serde_json::{Value, json};

use crate::cli::{Cli, CommandsArgs, JsonArg};
use crate::config::Config;
use crate::error::Error;

/// Every command, from the parsed command-line definition itself.
pub(crate) fn run(args: &CommandsArgs) -> Result<(), Error> {
    let mut root = Cli::command();
    root.build();
    let commands = list(&root, &[]);
    if args.json {
        return print_json(&json!({ "commands": commands }));
    }
    for command in &commands {
        let path = command["path"].as_str().unwrap_or_default();
        let about = command["about"].as_str().unwrap_or_default();
        println!("stack {path:<18} {about}");
    }
    println!("\nRun `stack <command> --help` for each one's arguments, `stack guide` for the manual.");
    Ok(())
}

/// `stack config show`: each setting, its value and where it came from.
pub(crate) fn config_show(args: &JsonArg) -> Result<(), Error> {
    let config = Config::load()?;
    if args.json {
        print_json(&config.to_json())
    } else {
        print!("{}", config.to_text());
        Ok(())
    }
}

fn print_json(value: &Value) -> Result<(), Error> {
    let text = serde_json::to_string_pretty(value).map_err(|source| Error::Json { source })?;
    println!("{text}");
    Ok(())
}

/// The runnable commands under `command`, depth first: a command with
/// subcommands is listed through them.
fn list(command: &clap::Command, parents: &[&str]) -> Vec<Value> {
    let mut found = Vec::new();
    for sub in command.get_subcommands().filter(|sub| sub.get_name() != "help") {
        let mut path: Vec<&str> = parents.to_vec();
        path.push(sub.get_name());
        if sub.has_subcommands() {
            found.extend(list(sub, &path));
        } else {
            found.push(describe(sub, &path));
        }
    }
    found
}

fn describe(command: &clap::Command, path: &[&str]) -> Value {
    let args: Vec<Value> = command
        .get_arguments()
        .filter(|arg| !matches!(arg.get_id().as_str(), "help" | "version"))
        .map(|arg| {
            let takes_value = arg.get_num_args().is_some_and(|range| range.takes_values());
            let defaults: Vec<String> = arg
                .get_default_values()
                .iter()
                .map(|value| value.to_string_lossy().into_owned())
                .collect();
            json!({
                "name": arg.get_long().map_or_else(|| arg.get_id().to_string(), |long| format!("--{long}")),
                "positional": arg.is_positional(),
                "value": takes_value.then(|| {
                    arg.get_value_names()
                        .and_then(|names| names.first())
                        .map_or_else(|| arg.get_id().to_string(), ToString::to_string)
                }),
                "required": arg.is_required_set(),
                "repeatable": matches!(arg.get_action(), clap::ArgAction::Append),
                "default": defaults.first(),
                "help": arg.get_help().map(ToString::to_string),
            })
        })
        .collect();
    json!({
        "path": path.join(" "),
        "about": command.get_about().map(ToString::to_string),
        "args": args,
    })
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory as _;

    use super::list;
    use crate::cli::Cli;

    /// Every leaf command of the CLI enum, as `["restack", "edit"]` paths.
    fn leaves(command: &clap::Command, parents: &[String]) -> Vec<Vec<String>> {
        command
            .get_subcommands()
            .filter(|sub| sub.get_name() != "help")
            .flat_map(|sub| {
                let mut path = parents.to_vec();
                path.push(sub.get_name().to_owned());
                if sub.has_subcommands() {
                    leaves(sub, &path)
                } else {
                    vec![path]
                }
            })
            .collect()
    }

    #[test]
    fn every_command_is_listed_and_in_its_parents_help() {
        let mut root = Cli::command();
        root.build();
        let listed: Vec<String> = list(&root, &[])
            .iter()
            .filter_map(|command| command["path"].as_str().map(str::to_owned))
            .collect();
        let all = leaves(&root, &[]);
        assert!(!all.is_empty());
        for path in &all {
            assert!(listed.contains(&path.join(" ")), "{path:?} not in {listed:?}");
            let mut parent = Cli::command();
            parent.build();
            for name in &path[..path.len() - 1] {
                parent = parent
                    .find_subcommand(name)
                    .cloned()
                    .unwrap_or_else(|| panic!("no {name}"));
            }
            let help = parent.render_long_help().to_string();
            let last = path.last().map(String::as_str).unwrap_or_default();
            assert!(help.contains(last), "{last} not in the help of {path:?}");
        }
    }

    #[test]
    fn every_listed_command_has_a_purpose_and_its_flags() {
        let mut root = Cli::command();
        root.build();
        for command in list(&root, &[]) {
            assert!(command["about"].is_string(), "{command}");
            for arg in command["args"].as_array().into_iter().flatten() {
                assert!(arg["help"].is_string(), "{command}: {arg}");
            }
        }
    }
}
