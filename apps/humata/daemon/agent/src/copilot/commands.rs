//! Running a skill or a plugin's command of Copilot's: a message that is
//! `/name input` and names one the session has is run through the CLI's
//! `session.commands.invoke` rather than sent as text, which Copilot would
//! read as words. A CLI without the call, and a name it doesn't list, get
//! the message as it is.

use serde_json::{Value, json};

use super::CopilotError;
use super::rpc::{Cli, METHOD_NOT_FOUND};

/// Copilot's calls that list a session's commands and run one.
const LIST: &str = "session.commands.list";
const INVOKE: &str = "session.commands.invoke";

/// What running a command came to.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Ran {
    /// A prompt for the agent, sent in place of the message, after telling
    /// the person `notice`, if any.
    Prompt {
        prompt: String,
        notice: Option<String>,
    },
    /// Done without the agent; what it said, if anything.
    Done(Option<String>),
}

/// Runs `text` as a command of `session`'s when it is `/name input` and
/// names one of its skills or plugin commands. None when it isn't, or the
/// CLI can't run commands: the message then goes as it is.
pub(super) async fn run(cli: &Cli, session: &str, text: &str) -> Result<Option<Ran>, CopilotError> {
    let Some((name, input)) = crate::slash::command(text) else {
        return Ok(None);
    };
    if !cli.has(INVOKE).await {
        return Ok(None);
    }
    // Copilot's own commands, such as /clear, change the session in ways
    // Humata's history doesn't follow, so only skills and plugins' are run.
    let filter = json!({
        "sessionId": session,
        "includeSkills": true,
        "includeBuiltins": false,
        "includeClientCommands": false,
    });
    let listed = match cli.call(LIST, filter).await {
        Ok(listed) => listed,
        Err(CopilotError::Rpc { .. }) => return Ok(None),
        Err(error) => return Err(error),
    };
    let Some(name) = listed_as(&listed, name) else {
        return Ok(None);
    };
    let mut params = json!({"sessionId": session, "name": name});
    if !input.is_empty() {
        params["input"] = json!(input);
    }
    match cli.call(INVOKE, params).await {
        Ok(result) => Ok(ran(&result)),
        Err(CopilotError::Rpc {
            code: METHOD_NOT_FOUND,
            ..
        }) => Ok(None),
        Err(error) => Err(error),
    }
}

/// The name Copilot lists the command `name` under, in its answer
/// `listed`: by its name or an alias, whatever the case. None for one it
/// doesn't list, or one of its own built in.
fn listed_as(listed: &Value, name: &str) -> Option<String> {
    let same = |other: &Value| {
        other
            .as_str()
            .is_some_and(|other| other.eq_ignore_ascii_case(name))
    };
    listed["commands"]
        .as_array()?
        .iter()
        .filter(|command| command["kind"] != "builtin")
        .find(|command| {
            same(&command["name"])
                || command["aliases"]
                    .as_array()
                    .is_some_and(|aliases| aliases.iter().any(same))
        })
        .and_then(|command| command["name"].as_str())
        .map(str::to_owned)
}

/// What Copilot's answer to running a command says came of it. None for
/// an answer of a kind this doesn't know, which is sent as text instead.
fn ran(result: &Value) -> Option<Ran> {
    let text = |field: &str| {
        result[field]
            .as_str()
            .filter(|text| !text.trim().is_empty())
            .map(str::to_owned)
    };
    match result["kind"].as_str()? {
        "agent-prompt" => Some(Ran::Prompt {
            prompt: text("prompt")?,
            notice: text("notice"),
        }),
        "text" => Some(Ran::Done(text("text"))),
        "completed" => Some(Ran::Done(text("message"))),
        "select-subcommand" => {
            let command = text("command").unwrap_or_default();
            let options: Vec<&str> = result["options"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|option| option["name"].as_str())
                .collect();
            Some(Ran::Done(Some(format!(
                "/{command} needs to be told which: {}. Send it again with one of them after its name.",
                options.join(", ")
            ))))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt as _;
    use std::path::Path;
    use std::sync::Arc;

    use super::super::rpc::PROTOCOL_VERSION;
    use super::*;

    /// A server that answers the hello and, with `commands`, lists the
    /// skill `release` (also `ship`) and the built-in `clear`, and answers
    /// running one with a prompt for the agent, keeping what it was asked
    /// to run in the file beside it; without, it has no call for commands.
    fn server(commands: bool) -> String {
        let no_method = r#"answer="{\"jsonrpc\":\"2.0\",\"id\":$id,\"error\":{\"code\":-32601,\"message\":\"no such method\"}}""#;
        let (list, invoke) = if commands {
            (
                r#"answer="{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{\"commands\":[{\"name\":\"release\",\"aliases\":[\"ship\"],\"kind\":\"skill\"},{\"name\":\"clear\",\"kind\":\"builtin\"}]}}""#,
                r#"printf '%s\n' "$body" >> "$0.invoked"
      answer="{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{\"kind\":\"agent-prompt\",\"prompt\":\"Cut the release\",\"displayPrompt\":\"/release\"}}""#,
            )
        } else {
            (no_method, no_method)
        };
        format!(
            r#"#!/bin/sh
while IFS= read -r header; do
  length=$(printf '%s' "$header" | tr -dc '0-9')
  IFS= read -r blank
  body=$(dd bs=1 count="$length" 2>/dev/null)
  id=$(printf '%s' "$body" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
  case "$body" in
    *'"method":"connect"'*)
      answer="{{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{{\"protocolVersion\":{PROTOCOL_VERSION}}}}}" ;;
    *'"method":"session.commands.list"'*)
      {list} ;;
    *'"method":"session.commands.invoke"'*'"name"'*)
      {invoke} ;;
    *'"method":"session.commands.invoke"'*)
      {probe} ;;
    *)
      answer="{{\"jsonrpc\":\"2.0\",\"id\":$id,\"error\":{{\"code\":-32000,\"message\":\"no such session\"}}}}" ;;
  esac
  printf 'Content-Length: %s\r\n\r\n%s' "${{#answer}}" "$answer"
done
"#,
            probe = if commands {
                r#"answer="{\"jsonrpc\":\"2.0\",\"id\":$id,\"error\":{\"code\":-32602,\"message\":\"name is required\"}}""#
            } else {
                no_method
            },
        )
    }

    async fn cli(folder: &Path, commands: bool) -> Arc<Cli> {
        let program = folder.join("server");
        std::fs::write(&program, server(commands)).unwrap_or_else(|error| panic!("{error}"));
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755))
            .unwrap_or_else(|error| panic!("{error}"));
        Cli::start(&program, "--no-experimental", &[], &folder.join("running"))
            .await
            .unwrap_or_else(|error| panic!("{error}"))
    }

    fn invoked(folder: &Path) -> String {
        std::fs::read_to_string(folder.join("server.invoked")).unwrap_or_default()
    }

    #[tokio::test]
    async fn a_listed_skill_is_run_with_its_input_and_gives_the_agent_its_prompt() {
        let dir = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
        let cli = cli(dir.path(), true).await;

        let ran = run(&cli, "session-1", "/Ship 1.2 today")
            .await
            .unwrap_or_else(|error| panic!("{error}"));

        assert_eq!(
            ran,
            Some(Ran::Prompt {
                prompt: "Cut the release".into(),
                notice: None
            })
        );
        let asked = invoked(dir.path());
        assert!(asked.contains(r#""name":"release""#), "{asked}");
        assert!(asked.contains(r#""input":"1.2 today""#), "{asked}");
    }

    #[tokio::test]
    async fn a_built_in_an_unlisted_name_and_plain_text_go_as_text() {
        let dir = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
        let cli = cli(dir.path(), true).await;

        for text in ["/clear", "/unknown thing", "release 1.2", "/usr/bin/env"] {
            let ran = run(&cli, "session-1", text)
                .await
                .unwrap_or_else(|error| panic!("{error}"));
            assert_eq!(ran, None, "{text}");
        }
        assert_eq!(invoked(dir.path()), "", "nothing was run");
    }

    #[tokio::test]
    async fn a_cli_without_commands_gets_the_message_as_text() {
        let dir = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
        let cli = cli(dir.path(), false).await;

        let ran = run(&cli, "session-1", "/release 1.2")
            .await
            .unwrap_or_else(|error| panic!("{error}"));

        assert_eq!(ran, None);
    }

    #[test]
    fn each_kind_of_answer_is_told_apart() {
        assert_eq!(
            ran(
                &json!({"kind": "agent-prompt", "prompt": "Do it", "displayPrompt": "/x", "notice": "Heads up"})
            ),
            Some(Ran::Prompt {
                prompt: "Do it".into(),
                notice: Some("Heads up".into())
            })
        );
        assert_eq!(
            ran(&json!({"kind": "text", "text": "Listed."})),
            Some(Ran::Done(Some("Listed.".into())))
        );
        assert_eq!(ran(&json!({"kind": "completed"})), Some(Ran::Done(None)));
        assert!(matches!(
            ran(
                &json!({"kind": "select-subcommand", "command": "x", "title": "Pick", "options": [{"name": "a", "description": ""}]})
            ),
            Some(Ran::Done(Some(_)))
        ));
        assert_eq!(ran(&json!({"kind": "something-new"})), None);
        assert_eq!(ran(&json!({"kind": "agent-prompt"})), None);
    }
}
