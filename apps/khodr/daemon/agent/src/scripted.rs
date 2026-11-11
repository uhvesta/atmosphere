//! The scripted backend: replies from a [`Script`], so tests are repeatable
//! and need no model.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::error::AgentError;
use crate::event::{EventKind, PermissionKind};
use crate::runtime::{Runner, Turn};

/// What the agent does for each message: the first rule whose `when` is in
/// the message plays its steps.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Script {
    pub rules: Vec<Rule>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rule {
    /// Matches a message that contains it; empty matches every message.
    #[serde(default)]
    pub when: String,
    pub steps: Vec<Step>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "step", rename_all = "snake_case")]
pub enum Step {
    /// Replies, streamed word by word.
    Say {
        text: String,
    },
    Think {
        text: String,
    },
    /// Asks to run `command` and, if allowed, reports `output` without
    /// running anything.
    Shell {
        command: String,
        output: String,
        #[serde(default = "succeeds")]
        success: bool,
    },
    /// Asks to write `contents` to `path`, inside the working directory, and
    /// writes it if allowed.
    WriteFile {
        path: PathBuf,
        contents: String,
    },
    /// Fails the turn.
    Fail {
        message: String,
    },
}

fn succeeds() -> bool {
    true
}

#[derive(Debug, thiserror::Error)]
pub enum ScriptedError {
    #[error(
        "the script has no rule for the message {message:?}; add a rule whose `when` it contains, or one with an empty `when`"
    )]
    NoRule { message: String },

    #[error(
        "the session's script isn't valid; fix the JSON given as its backend config and start a new session with it"
    )]
    BadScript {
        #[source]
        source: serde_json::Error,
    },

    #[error(
        "the script writes to {path}, outside the session's working directory; use a relative path without `..`"
    )]
    OutsideWorkingDirectory { path: PathBuf },

    #[error("couldn't write {path}; check that the session's working directory is writable")]
    WriteFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("{message}")]
    Scripted { message: String },

    #[error(transparent)]
    Agent(#[from] AgentError),
}

/// Plays each session's own script, from its backend config, or else the
/// one it was made with.
pub struct ScriptedRunner {
    script: Script,
}

impl ScriptedRunner {
    pub fn new(script: Script) -> Self {
        Self { script }
    }
}

impl Runner for ScriptedRunner {
    type Error = ScriptedError;

    async fn run_turn(self: Arc<Self>, turn: Turn) -> Result<(), ScriptedError> {
        let own = match &turn.record().backend_config {
            Some(json) => Some(
                serde_json::from_str::<Script>(json)
                    .map_err(|source| ScriptedError::BadScript { source })?,
            ),
            None => None,
        };
        let script = own.as_ref().unwrap_or(&self.script);
        let rule = script
            .rules
            .iter()
            .find(|rule| turn.text().contains(&rule.when))
            .ok_or_else(|| ScriptedError::NoRule {
                message: turn.text().to_owned(),
            })?;
        for (index, step) in rule.steps.iter().enumerate() {
            play(&turn, index, step).await?;
        }
        Ok(())
    }
}

async fn play(turn: &Turn, index: usize, step: &Step) -> Result<(), ScriptedError> {
    let call_id = format!("call-{index}");
    match step {
        Step::Say { text } => {
            for word in text.split_inclusive(' ') {
                turn.emit(EventKind::AssistantDelta {
                    text: word.to_owned(),
                })?;
                // Lets watchers see it stream, and an abort land mid-reply.
                tokio::task::yield_now().await;
            }
            turn.emit(EventKind::AssistantMessage { text: text.clone() })?;
        }
        Step::Think { text } => {
            turn.emit(EventKind::Reasoning { text: text.clone() })?;
        }
        Step::Shell {
            command,
            output,
            success,
        } => {
            turn.emit(EventKind::ToolStarted {
                call_id: call_id.clone(),
                tool: "shell".into(),
                arguments_json: serde_json::json!({ "command": command }).to_string(),
            })?;
            let allowed = turn
                .ask(PermissionKind::Shell, format!("Run `{command}`"))
                .await?;
            let (success, output) = if allowed {
                (*success, output.clone())
            } else {
                (false, denied())
            };
            turn.emit(EventKind::ToolCompleted {
                call_id,
                success,
                output,
            })?;
        }
        Step::WriteFile { path, contents } => {
            let target = inside(&turn.record().working_directory, path)?;
            turn.emit(EventKind::ToolStarted {
                call_id: call_id.clone(),
                tool: "write_file".into(),
                arguments_json: serde_json::json!({ "path": path }).to_string(),
            })?;
            let allowed = turn
                .ask(PermissionKind::Write, format!("Write {}", path.display()))
                .await?;
            if !allowed {
                turn.emit(EventKind::ToolCompleted {
                    call_id,
                    success: false,
                    output: denied(),
                })?;
                return Ok(());
            }
            write(&target, contents).await?;
            turn.emit(EventKind::ToolCompleted {
                call_id,
                success: true,
                output: format!("Wrote {}", path.display()),
            })?;
        }
        Step::Fail { message } => {
            return Err(ScriptedError::Scripted {
                message: message.clone(),
            });
        }
    }
    Ok(())
}

fn denied() -> String {
    "The person denied this.".into()
}

fn inside(root: &Path, path: &Path) -> Result<PathBuf, ScriptedError> {
    crate::paths::inside(root, path).ok_or_else(|| ScriptedError::OutsideWorkingDirectory {
        path: path.to_owned(),
    })
}

async fn write(target: &Path, contents: &str) -> Result<(), ScriptedError> {
    let failed = |source| ScriptedError::WriteFile {
        path: target.to_owned(),
        source,
    };
    if let Some(parent) = target.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(failed)?;
    }
    tokio::fs::write(target, contents).await.map_err(failed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{Event, ForkOptions, SendMode, SessionId, SessionState, StartOptions};
    use crate::runtime::tests::{idle_turns, permission_request, wait_for, waiting};
    use crate::{Agent, LocalAgent};

    struct Fixture {
        agent: LocalAgent<ScriptedRunner>,
        state: tempfile::TempDir,
        work: tempfile::TempDir,
    }

    fn fixture(rules: Vec<Rule>) -> Fixture {
        let state = tempfile::tempdir().expect("a temporary folder");
        let work = tempfile::tempdir().expect("a temporary folder");
        let agent = LocalAgent::open(state.path(), ScriptedRunner::new(Script { rules }))
            .expect("the agent opens");
        Fixture { agent, state, work }
    }

    impl Fixture {
        async fn start(&self) -> SessionId {
            let options = StartOptions {
                name: "test".into(),
                working_directory: self.work.path().to_owned(),
                ..StartOptions::default()
            };
            self.agent
                .start(options)
                .await
                .expect("the session starts")
                .id
        }
    }

    fn rule(when: &str, steps: Vec<Step>) -> Rule {
        Rule {
            when: when.into(),
            steps,
        }
    }

    fn say(text: &str) -> Step {
        Step::Say { text: text.into() }
    }

    fn write_file(path: &str, contents: &str) -> Step {
        Step::WriteFile {
            path: path.into(),
            contents: contents.into(),
        }
    }

    fn kinds(events: &[Event]) -> Vec<EventKind> {
        events.iter().map(|event| event.kind.clone()).collect()
    }

    fn user_message(events: &[Event], text: &str) -> Event {
        events
            .iter()
            .find(|event| event.kind == EventKind::UserMessage { text: text.into() })
            .cloned()
            .expect("the message is in the history")
    }

    #[tokio::test]
    async fn a_turn_streams_the_reply_and_ends_idle() {
        let fixture = fixture(vec![rule("", vec![say("hello there")])]);
        let id = fixture.start().await;
        fixture
            .agent
            .send(&id, "hi".into(), SendMode::Enqueue)
            .await
            .expect("sent");
        let events = wait_for(&fixture.agent, &id, idle_turns(1)).await;
        assert_eq!(
            kinds(&events),
            vec![
                EventKind::UserMessage { text: "hi".into() },
                EventKind::TurnStarted,
                EventKind::AssistantDelta {
                    text: "hello ".into()
                },
                EventKind::AssistantDelta {
                    text: "there".into()
                },
                EventKind::AssistantMessage {
                    text: "hello there".into()
                },
                EventKind::TurnEnded { aborted: false },
                EventKind::Idle,
            ]
        );
    }

    #[tokio::test]
    async fn an_approved_write_writes_the_file() {
        let fixture = fixture(vec![rule("", vec![write_file("src/a.txt", "one")])]);
        let id = fixture.start().await;
        fixture
            .agent
            .send(&id, "write".into(), SendMode::Enqueue)
            .await
            .expect("sent");
        let events = wait_for(&fixture.agent, &id, waiting).await;
        assert!(
            !fixture.work.path().join("src/a.txt").exists(),
            "nothing is written before approval"
        );
        let request = permission_request(&events).expect("a permission request");
        fixture
            .agent
            .approve(&id, &request, true)
            .await
            .expect("approved");
        wait_for(&fixture.agent, &id, idle_turns(1)).await;
        let written =
            std::fs::read_to_string(fixture.work.path().join("src/a.txt")).expect("written");
        assert_eq!(written, "one");
    }

    #[tokio::test]
    async fn a_denied_write_leaves_the_file_alone() {
        let fixture = fixture(vec![rule("", vec![write_file("a.txt", "one")])]);
        let id = fixture.start().await;
        fixture
            .agent
            .send(&id, "write".into(), SendMode::Enqueue)
            .await
            .expect("sent");
        let events = wait_for(&fixture.agent, &id, waiting).await;
        let request = permission_request(&events).expect("a permission request");
        fixture
            .agent
            .approve(&id, &request, false)
            .await
            .expect("denied");
        let events = wait_for(&fixture.agent, &id, idle_turns(1)).await;
        assert!(!fixture.work.path().join("a.txt").exists());
        assert!(
            events
                .iter()
                .any(|event| matches!(event.kind, EventKind::ToolCompleted { success: false, .. }))
        );
    }

    #[tokio::test]
    async fn a_write_outside_the_working_directory_fails_the_turn() {
        let fixture = fixture(vec![rule("", vec![write_file("../escape.txt", "x")])]);
        let id = fixture.start().await;
        fixture
            .agent
            .send(&id, "write".into(), SendMode::Enqueue)
            .await
            .expect("sent");
        let events = wait_for(&fixture.agent, &id, idle_turns(1)).await;
        assert_eq!(SessionState::after(&events), SessionState::Failed);
    }

    #[tokio::test]
    async fn answering_an_unknown_request_is_an_error() {
        let fixture = fixture(vec![]);
        let id = fixture.start().await;
        let result = fixture.agent.approve(&id, "nope", true).await;
        assert!(matches!(
            result,
            Err(AgentError::UnknownPermissionRequest { .. })
        ));
    }

    #[tokio::test]
    async fn a_message_without_a_rule_fails_the_turn() {
        let fixture = fixture(vec![rule("only this", vec![say("ok")])]);
        let id = fixture.start().await;
        fixture
            .agent
            .send(&id, "something else".into(), SendMode::Enqueue)
            .await
            .expect("sent");
        let events = wait_for(&fixture.agent, &id, idle_turns(1)).await;
        assert_eq!(SessionState::after(&events), SessionState::Failed);
    }

    #[tokio::test]
    async fn a_queued_message_runs_after_the_turn() {
        let fixture = fixture(vec![
            rule("first", vec![write_file("a.txt", "a")]),
            rule("second", vec![say("two")]),
        ]);
        let id = fixture.start().await;
        fixture
            .agent
            .send(&id, "first".into(), SendMode::Enqueue)
            .await
            .expect("sent");
        let events = wait_for(&fixture.agent, &id, waiting).await;
        fixture
            .agent
            .send(&id, "second".into(), SendMode::Enqueue)
            .await
            .expect("queued");
        let request = permission_request(&events).expect("a permission request");
        fixture
            .agent
            .approve(&id, &request, true)
            .await
            .expect("approved");
        let events = wait_for(&fixture.agent, &id, idle_turns(1)).await;
        let texts: Vec<_> = events
            .iter()
            .filter_map(|event| match &event.kind {
                EventKind::UserMessage { text } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(texts, vec!["first", "second"]);
        let starts = events
            .iter()
            .filter(|event| event.kind == EventKind::TurnStarted)
            .count();
        assert_eq!(starts, 2);
    }

    #[tokio::test]
    async fn aborting_a_turn_waiting_for_approval_writes_nothing() {
        let fixture = fixture(vec![rule("", vec![write_file("a.txt", "a")])]);
        let id = fixture.start().await;
        fixture
            .agent
            .send(&id, "write".into(), SendMode::Enqueue)
            .await
            .expect("sent");
        let events = wait_for(&fixture.agent, &id, waiting).await;
        fixture.agent.abort(&id).await.expect("aborted");
        let history = fixture.agent.history(&id).await.expect("history");
        assert_eq!(
            kinds(&history[history.len() - 2..]),
            vec![EventKind::TurnEnded { aborted: true }, EventKind::Idle]
        );
        assert_eq!(SessionState::after(&history), SessionState::Idle);
        assert!(!fixture.work.path().join("a.txt").exists());
        let request = permission_request(&events).expect("a permission request");
        let late = fixture.agent.approve(&id, &request, true).await;
        assert!(matches!(
            late,
            Err(AgentError::UnknownPermissionRequest { .. })
        ));
    }

    #[tokio::test]
    async fn a_fork_before_a_turn_has_the_history_up_to_it() {
        let fixture = fixture(vec![
            rule("one", vec![say("first")]),
            rule("two", vec![say("second")]),
        ]);
        let id = fixture.start().await;
        fixture
            .agent
            .send(&id, "one".into(), SendMode::Enqueue)
            .await
            .expect("sent");
        wait_for(&fixture.agent, &id, idle_turns(1)).await;
        fixture
            .agent
            .send(&id, "two".into(), SendMode::Enqueue)
            .await
            .expect("sent");
        let original = wait_for(&fixture.agent, &id, idle_turns(2)).await;
        let second = user_message(&original, "two");

        let options = ForkOptions {
            before: Some(second.id.clone()),
            ..ForkOptions::default()
        };
        let fork = fixture.agent.fork(&id, options).await.expect("forked");
        let forked = fixture.agent.history(&fork.id).await.expect("history");

        let cut = original
            .iter()
            .position(|event| event.id == second.id)
            .expect("in the history");
        assert_eq!(forked, original[..cut].to_vec());
        assert_eq!(
            fork.parent.as_ref().map(|parent| &parent.session),
            Some(&id)
        );
        assert_eq!(fork.working_directory, fixture.work.path());

        fixture
            .agent
            .send(&fork.id, "two".into(), SendMode::Enqueue)
            .await
            .expect("sent");
        wait_for(&fixture.agent, &fork.id, idle_turns(2)).await;
        let after = fixture.agent.history(&id).await.expect("history");
        assert_eq!(after, original, "the original is untouched");
    }

    #[tokio::test]
    async fn forking_before_an_unknown_event_is_an_error() {
        let fixture = fixture(vec![]);
        let id = fixture.start().await;
        let options = ForkOptions {
            before: Some(crate::EventId::new("nope")),
            ..ForkOptions::default()
        };
        let result = fixture.agent.fork(&id, options).await;
        assert!(matches!(result, Err(AgentError::UnknownEvent { .. })));
    }

    #[tokio::test]
    async fn sessions_survive_reopening() {
        let fixture = fixture(vec![rule("", vec![say("kept")])]);
        let id = fixture.start().await;
        fixture
            .agent
            .send(&id, "hi".into(), SendMode::Enqueue)
            .await
            .expect("sent");
        let before = wait_for(&fixture.agent, &id, idle_turns(1)).await;

        let reopened =
            LocalAgent::open(fixture.state.path(), ScriptedRunner::new(Script::default()))
                .expect("reopens");
        let after = reopened.history(&id).await.expect("history");
        assert_eq!(after, before);
        let ids: Vec<_> = reopened
            .sessions()
            .await
            .expect("sessions")
            .into_iter()
            .map(|s| s.id)
            .collect();
        assert_eq!(ids, vec![id]);
    }

    #[tokio::test]
    async fn a_session_cut_off_mid_turn_reopens_aborted() {
        let fixture = fixture(vec![rule("", vec![write_file("a.txt", "a")])]);
        let id = fixture.start().await;
        fixture
            .agent
            .send(&id, "write".into(), SendMode::Enqueue)
            .await
            .expect("sent");
        wait_for(&fixture.agent, &id, waiting).await;

        let reopened =
            LocalAgent::open(fixture.state.path(), ScriptedRunner::new(Script::default()))
                .expect("reopens");
        let history = reopened.history(&id).await.expect("history");
        assert_eq!(SessionState::after(&history), SessionState::Idle);
        assert!(
            history
                .iter()
                .any(|event| event.kind == EventKind::TurnEnded { aborted: true })
        );
    }

    #[tokio::test]
    async fn a_session_plays_its_own_script() {
        let fixture = fixture(vec![rule("", vec![say("default")])]);
        let script = Script {
            rules: vec![rule("", vec![say("own")])],
        };
        let options = StartOptions {
            working_directory: fixture.work.path().to_owned(),
            backend_config: Some(serde_json::to_string(&script).expect("encodes")),
            ..StartOptions::default()
        };
        let id = fixture.agent.start(options).await.expect("starts").id;
        fixture
            .agent
            .send(&id, "hi".into(), SendMode::Enqueue)
            .await
            .expect("sent");
        let events = wait_for(&fixture.agent, &id, idle_turns(1)).await;
        assert!(
            events
                .iter()
                .any(|event| event.kind == EventKind::AssistantMessage { text: "own".into() })
        );
    }

    #[test]
    fn scripts_read_from_json() {
        let json = r#"{"rules": [{"when": "fix", "steps": [
            {"step": "think", "text": "hmm"},
            {"step": "shell", "command": "ls", "output": "a"},
            {"step": "write_file", "path": "a.txt", "contents": "x"}
        ]}]}"#;
        let script: Script = serde_json::from_str(json).expect("parses");
        assert_eq!(script.rules[0].steps.len(), 3);
        assert_eq!(
            script.rules[0].steps[1],
            Step::Shell {
                command: "ls".into(),
                output: "a".into(),
                success: true
            }
        );
    }
}
