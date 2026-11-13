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
    /// Calls `tool` with `arguments`, as an agent calls one of Humata's
    /// MCP tools, and reports `output`; nothing runs. Tests use it to stand
    /// in for an agent proposing tasks or answering threads.
    Tool {
        tool: String,
        #[serde(default)]
        arguments: serde_json::Value,
        #[serde(default)]
        output: String,
    },
    /// Fails the turn.
    Fail {
        message: String,
    },
    /// Holds the turn, working, until `path` exists inside the working
    /// directory; for tests that look at a session mid-turn.
    WaitFor {
        path: PathBuf,
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
        Step::Tool {
            tool,
            arguments,
            output,
        } => {
            turn.emit(EventKind::ToolStarted {
                call_id: call_id.clone(),
                tool: tool.clone(),
                arguments_json: arguments.to_string(),
            })?;
            turn.emit(EventKind::ToolCompleted {
                call_id,
                success: true,
                output: output.clone(),
            })?;
        }
        Step::Fail { message } => {
            return Err(ScriptedError::Scripted {
                message: message.clone(),
            });
        }
        Step::WaitFor { path } => {
            let target = inside(&turn.record().working_directory, path)?;
            while !tokio::fs::try_exists(&target).await.unwrap_or(false) {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
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
    use crate::event::{
        ApprovalMode, Event, ForkOptions, SendMode, SessionId, SessionState, StartOptions,
    };
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
            self.start_with(ApprovalMode::AskEveryTime).await
        }

        async fn start_with(&self, approval: ApprovalMode) -> SessionId {
            let options = StartOptions {
                name: "test".into(),
                working_directory: self.work.path().to_owned(),
                approval,
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
            .find(|event| {
                event.kind
                    == EventKind::UserMessage {
                        text: text.into(),
                        images: Vec::new(),
                    }
            })
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
                EventKind::UserMessage {
                    text: "hi".into(),
                    images: Vec::new()
                },
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

    fn shell(command: &str) -> Step {
        Step::Shell {
            command: command.into(),
            output: "ran".into(),
            success: true,
        }
    }

    /// Whether each permission request was answered automatically, in order.
    fn answers(events: &[Event]) -> Vec<(bool, bool)> {
        events
            .iter()
            .filter_map(|event| match event.kind {
                EventKind::PermissionCompleted {
                    approved,
                    automatic,
                    ..
                } => Some((approved, automatic)),
                _ => None,
            })
            .collect()
    }

    #[tokio::test]
    async fn allowing_edits_writes_without_asking_but_asks_for_commands() {
        let fixture = fixture(vec![rule(
            "",
            vec![write_file("a.txt", "one"), shell("make")],
        )]);
        let id = fixture.start_with(ApprovalMode::AllowEdits).await;
        fixture
            .agent
            .send(&id, "go".into(), SendMode::Enqueue)
            .await
            .expect("sent");
        let asks_to_run = |events: &[Event]| {
            events.iter().any(|event| {
                matches!(
                    event.kind,
                    EventKind::PermissionRequested {
                        kind: PermissionKind::Shell,
                        ..
                    }
                )
            })
        };
        let events = wait_for(&fixture.agent, &id, asks_to_run).await;
        let written = std::fs::read_to_string(fixture.work.path().join("a.txt")).expect("written");
        assert_eq!(written, "one");
        assert_eq!(answers(&events), vec![(true, true)]);
        let request = permission_request(&events).expect("the command asks");
        fixture
            .agent
            .approve(&id, &request, true)
            .await
            .expect("approved");
        let events = wait_for(&fixture.agent, &id, idle_turns(1)).await;
        assert_eq!(answers(&events), vec![(true, true), (true, false)]);
    }

    fn queued_ids(events: &[Event]) -> Vec<String> {
        let mut queued = Vec::new();
        for event in events {
            match &event.kind {
                EventKind::MessageQueued { queue_id, .. } if !queued.contains(queue_id) => {
                    queued.push(queue_id.clone())
                }
                EventKind::Dequeued { queue_id, .. } => queued.retain(|id| id != queue_id),
                _ => {}
            }
        }
        queued
    }

    #[tokio::test]
    async fn an_unqueued_message_never_runs() {
        let fixture = fixture(vec![rule("", vec![write_file("a.txt", "one")])]);
        let id = fixture.start().await;
        fixture
            .agent
            .send(&id, "first".into(), SendMode::Enqueue)
            .await
            .expect("sent");
        wait_for(&fixture.agent, &id, waiting).await;
        fixture
            .agent
            .send(&id, "second".into(), SendMode::Enqueue)
            .await
            .expect("queued");
        let events = wait_for(&fixture.agent, &id, |events| !queued_ids(events).is_empty()).await;
        let queued = queued_ids(&events);
        assert_eq!(queued.len(), 1);
        fixture
            .agent
            .unqueue(&id, &queued[0])
            .await
            .expect("unqueued");
        let request = permission_request(&events).expect("a permission request");
        fixture
            .agent
            .approve(&id, &request, true)
            .await
            .expect("approved");
        let events = wait_for(&fixture.agent, &id, idle_turns(1)).await;
        assert!(queued_ids(&events).is_empty());
        assert!(!events.iter().any(|event| event.kind
            == EventKind::UserMessage {
                text: "second".into(),
                images: Vec::new(),
            }));
        let again = fixture.agent.unqueue(&id, &queued[0]).await;
        assert!(matches!(
            again,
            Err(AgentError::UnknownQueuedMessage { .. })
        ));
    }

    /// Whether `count` turns have ended; queued turns run one after
    /// another with no idle between them.
    fn ended_turns(count: usize) -> impl Fn(&[Event]) -> bool {
        move |events| {
            events
                .iter()
                .filter(|event| matches!(event.kind, EventKind::TurnEnded { .. }))
                .count()
                >= count
        }
    }

    fn user_messages(events: &[Event]) -> Vec<String> {
        events
            .iter()
            .filter_map(|event| match &event.kind {
                EventKind::UserMessage { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    /// Starts a turn that waits for a permission answer, queues `queued`
    /// behind it, and gives the permission request.
    async fn queue_behind_a_turn(fixture: &Fixture, id: &SessionId, queued: &[&str]) -> String {
        fixture
            .agent
            .send(id, "first".into(), SendMode::Enqueue)
            .await
            .expect("sent");
        let events = wait_for(&fixture.agent, id, waiting).await;
        for text in queued {
            fixture
                .agent
                .send(id, (*text).into(), SendMode::Enqueue)
                .await
                .expect("queued");
        }
        permission_request(&events).expect("a permission request")
    }

    #[tokio::test]
    async fn messages_sent_while_one_waits_run_as_one() {
        let fixture = fixture(vec![
            rule("first", vec![write_file("a.txt", "one")]),
            rule("", vec![say("done")]),
        ]);
        let id = fixture.start().await;
        let request = queue_behind_a_turn(&fixture, &id, &["second", "third"]).await;
        let events = wait_for(&fixture.agent, &id, |events| !queued_ids(events).is_empty()).await;
        assert_eq!(queued_ids(&events).len(), 1);
        fixture
            .agent
            .approve(&id, &request, true)
            .await
            .expect("approved");
        let events = wait_for(&fixture.agent, &id, ended_turns(2)).await;
        assert_eq!(user_messages(&events), vec!["first", "second\n\nthird"]);
    }

    #[tokio::test]
    async fn an_edited_queued_message_runs_as_edited() {
        let fixture = fixture(vec![
            rule("first", vec![write_file("a.txt", "one")]),
            rule("", vec![say("done")]),
        ]);
        let id = fixture.start().await;
        let request = queue_behind_a_turn(&fixture, &id, &["second"]).await;
        let events = wait_for(&fixture.agent, &id, |events| !queued_ids(events).is_empty()).await;
        let queued = queued_ids(&events);
        fixture
            .agent
            .edit_queued(&id, &queued[0], "edited".into())
            .await
            .expect("edited");
        fixture
            .agent
            .approve(&id, &request, true)
            .await
            .expect("approved");
        let events = wait_for(&fixture.agent, &id, ended_turns(2)).await;
        assert_eq!(user_messages(&events), vec!["first", "edited"]);
        let late = fixture
            .agent
            .edit_queued(&id, &queued[0], "too late".into())
            .await;
        assert!(matches!(late, Err(AgentError::UnknownQueuedMessage { .. })));
    }

    fn png(byte: u8) -> crate::event::Image {
        crate::event::Image {
            media_type: "image/png".into(),
            data: vec![byte],
        }
    }

    fn with_images(text: &str, images: Vec<crate::event::Image>) -> crate::event::Message {
        crate::event::Message {
            text: text.into(),
            images,
        }
    }

    /// The image names of each message the person sent, in order.
    fn sent_images(events: &[Event]) -> Vec<Vec<String>> {
        events
            .iter()
            .filter_map(|event| match &event.kind {
                EventKind::UserMessage { images, .. } => Some(images.clone()),
                _ => None,
            })
            .collect()
    }

    #[tokio::test]
    async fn images_sent_with_a_message_are_kept_and_read_back() {
        let fixture = fixture(vec![rule("", vec![say("a cat")])]);
        let id = fixture.start().await;
        fixture
            .agent
            .send(
                &id,
                with_images("what is it?", vec![png(1), png(2)]),
                SendMode::Enqueue,
            )
            .await
            .expect("sent");
        let events = wait_for(&fixture.agent, &id, idle_turns(1)).await;
        let names = sent_images(&events).concat();
        assert_eq!(names.len(), 2);
        let first = fixture
            .agent
            .read_image(&id, &names[0])
            .await
            .expect("read");
        let second = fixture
            .agent
            .read_image(&id, &names[1])
            .await
            .expect("read");
        assert_eq!((first, second), (png(1), png(2)));
        let unknown = fixture.agent.read_image(&id, "nope.png").await;
        assert!(matches!(unknown, Err(AgentError::UnknownImage { .. })));
    }

    #[tokio::test]
    async fn an_image_of_a_kind_the_agents_cant_read_is_refused() {
        let fixture = fixture(vec![]);
        let id = fixture.start().await;
        let tiff = crate::event::Image {
            media_type: "image/tiff".into(),
            data: vec![1],
        };
        let sent = fixture
            .agent
            .send(&id, with_images("look", vec![tiff]), SendMode::Enqueue)
            .await;
        assert!(matches!(sent, Err(AgentError::UnsupportedImage { .. })));
    }

    #[tokio::test]
    async fn queued_messages_that_run_as_one_keep_all_their_images() {
        let fixture = fixture(vec![
            rule("first", vec![write_file("a.txt", "one")]),
            rule("", vec![say("done")]),
        ]);
        let id = fixture.start().await;
        let request = queue_behind_a_turn(&fixture, &id, &[]).await;
        for (text, byte) in [("second", 1), ("third", 2)] {
            fixture
                .agent
                .send(&id, with_images(text, vec![png(byte)]), SendMode::Enqueue)
                .await
                .expect("queued");
        }
        fixture
            .agent
            .approve(&id, &request, true)
            .await
            .expect("approved");
        let events = wait_for(&fixture.agent, &id, ended_turns(2)).await;
        let images = sent_images(&events);
        assert_eq!(images.len(), 2);
        assert_eq!(
            images[1].len(),
            2,
            "both queued images go with the joined message"
        );
    }

    #[tokio::test]
    async fn a_fork_can_still_read_its_parents_images() {
        let fixture = fixture(vec![rule("", vec![say("a cat")])]);
        let id = fixture.start().await;
        fixture
            .agent
            .send(
                &id,
                with_images("what is it?", vec![png(7)]),
                SendMode::Enqueue,
            )
            .await
            .expect("sent");
        let events = wait_for(&fixture.agent, &id, idle_turns(1)).await;
        let name = sent_images(&events).concat().remove(0);
        let fork = fixture
            .agent
            .fork(&id, ForkOptions::default())
            .await
            .expect("forked");
        let image = fixture
            .agent
            .read_image(&fork.id, &name)
            .await
            .expect("read");
        assert_eq!(image, png(7));
    }

    #[tokio::test]
    async fn a_notice_never_joins_a_queued_message() {
        let fixture = fixture(vec![
            rule("first", vec![write_file("a.txt", "one")]),
            rule("", vec![say("done")]),
        ]);
        let id = fixture.start().await;
        let request = queue_behind_a_turn(&fixture, &id, &["second"]).await;
        fixture
            .agent
            .send(&id, "the spec changed".into(), SendMode::EnqueueOnce)
            .await
            .expect("queued");
        fixture
            .agent
            .approve(&id, &request, true)
            .await
            .expect("approved");
        let events = wait_for(&fixture.agent, &id, ended_turns(3)).await;
        assert_eq!(
            user_messages(&events),
            vec!["first", "second", "the spec changed"]
        );
    }

    #[tokio::test]
    async fn a_notice_sent_again_while_it_waits_starts_one_turn() {
        let fixture = fixture(vec![rule("", vec![write_file("a.txt", "one")])]);
        let id = fixture.start().await;
        fixture
            .agent
            .send(&id, "first".into(), SendMode::Enqueue)
            .await
            .expect("sent");
        wait_for(&fixture.agent, &id, waiting).await;
        for _ in 0..3 {
            fixture
                .agent
                .send(&id, "the spec changed".into(), SendMode::EnqueueOnce)
                .await
                .expect("queued");
        }
        let events = wait_for(&fixture.agent, &id, |events| !queued_ids(events).is_empty()).await;
        assert_eq!(queued_ids(&events).len(), 1);
        fixture
            .agent
            .send(&id, "another".into(), SendMode::EnqueueOnce)
            .await
            .expect("queued");
        let events = wait_for(&fixture.agent, &id, |events| queued_ids(events).len() == 2).await;
        assert_eq!(queued_ids(&events).len(), 2);
    }

    #[tokio::test]
    async fn allowing_everything_never_waits() {
        let fixture = fixture(vec![rule(
            "",
            vec![shell("make"), write_file("a.txt", "one")],
        )]);
        let id = fixture.start_with(ApprovalMode::AllowEverything).await;
        fixture
            .agent
            .send(&id, "go".into(), SendMode::Enqueue)
            .await
            .expect("sent");
        let events = wait_for(&fixture.agent, &id, idle_turns(1)).await;
        assert_eq!(answers(&events), vec![(true, true), (true, true)]);
        assert!(fixture.work.path().join("a.txt").exists());
    }

    #[tokio::test]
    async fn changing_the_mode_answers_requests_it_allows() {
        let fixture = fixture(vec![rule("", vec![write_file("a.txt", "one")])]);
        let id = fixture.start().await;
        fixture
            .agent
            .send(&id, "go".into(), SendMode::Enqueue)
            .await
            .expect("sent");
        wait_for(&fixture.agent, &id, waiting).await;
        fixture
            .agent
            .set_approval_mode(&id, ApprovalMode::AllowEdits)
            .await
            .expect("set");
        let events = wait_for(&fixture.agent, &id, idle_turns(1)).await;
        assert_eq!(answers(&events), vec![(true, true)]);
        assert!(fixture.work.path().join("a.txt").exists());
    }

    #[tokio::test]
    async fn in_auto_what_reaches_humata_still_asks_the_person() {
        let fixture = fixture(vec![rule("", vec![write_file("a.txt", "one")])]);
        let id = fixture.start().await;
        fixture
            .agent
            .send(&id, "go".into(), SendMode::Enqueue)
            .await
            .expect("sent");
        wait_for(&fixture.agent, &id, waiting).await;
        fixture
            .agent
            .set_approval_mode(&id, ApprovalMode::Auto)
            .await
            .expect("set");
        let events = wait_for(&fixture.agent, &id, waiting).await;
        assert!(answers(&events).is_empty());
        let request = permission_request(&events).expect("still waiting");
        fixture
            .agent
            .approve(&id, &request, true)
            .await
            .expect("approved");
        let events = wait_for(&fixture.agent, &id, idle_turns(1)).await;
        assert_eq!(answers(&events), vec![(true, false)]);
    }

    #[tokio::test]
    async fn the_mode_survives_reopening_and_forks_keep_it() {
        let fixture = fixture(vec![rule("", vec![say("hi")])]);
        let id = fixture.start().await;
        fixture
            .agent
            .set_approval_mode(&id, ApprovalMode::AllowEverything)
            .await
            .expect("set");
        let fork = fixture
            .agent
            .fork(&id, ForkOptions::default())
            .await
            .expect("forked");
        assert_eq!(fork.approval, ApprovalMode::AllowEverything);
        let reopened =
            LocalAgent::open(fixture.state.path(), ScriptedRunner::new(Script::default()))
                .expect("reopens");
        let modes: Vec<_> = reopened
            .sessions()
            .await
            .expect("sessions")
            .into_iter()
            .map(|record| record.approval)
            .collect();
        assert_eq!(
            modes,
            vec![ApprovalMode::AllowEverything, ApprovalMode::AllowEverything]
        );
    }

    #[tokio::test]
    async fn a_damaged_session_is_listed_with_why_and_can_be_removed() {
        let fixture = fixture(vec![rule("", vec![say("hi")])]);
        let kept = fixture.start().await;
        let damaged = fixture.start().await;
        let folder = fixture.state.path().join("sessions").join(damaged.as_str());
        std::fs::write(folder.join("session.json"), "{ not json").expect("damage the record");

        let reopened =
            LocalAgent::open(fixture.state.path(), ScriptedRunner::new(Script::default()))
                .expect("one damaged session doesn't keep the rest closed");
        let open: Vec<_> = reopened
            .sessions()
            .await
            .expect("sessions")
            .into_iter()
            .map(|record| record.id)
            .collect();
        assert_eq!(open, vec![kept]);
        let skipped = reopened.skipped().await.expect("skipped");
        assert_eq!(
            skipped
                .iter()
                .map(|skipped| skipped.id.as_str())
                .collect::<Vec<_>>(),
            vec![damaged.as_str()]
        );
        assert!(!skipped[0].reason.is_empty());

        assert!(matches!(
            reopened.remove_skipped("not-one").await,
            Err(AgentError::UnknownSkipped { .. })
        ));
        reopened
            .remove_skipped(damaged.as_str())
            .await
            .expect("removed");
        assert!(!folder.exists());
        assert!(reopened.skipped().await.expect("skipped").is_empty());
        assert!(matches!(
            reopened.remove_skipped(damaged.as_str()).await,
            Err(AgentError::UnknownSkipped { .. })
        ));
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
                EventKind::UserMessage { text, .. } => Some(text.as_str()),
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
        assert_eq!(after, whole(original), "the original is untouched");
    }

    /// `events` as a history keeps them once its replies are whole: without
    /// the parts they streamed in.
    fn whole(events: Vec<Event>) -> Vec<Event> {
        events
            .into_iter()
            .filter(|event| !matches!(event.kind, EventKind::AssistantDelta { .. }))
            .collect()
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
        assert_eq!(after, whole(before));
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
    async fn a_session_cut_off_mid_turn_reopens_aborted_and_failed() {
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
        assert_eq!(SessionState::after(&history), SessionState::Failed);
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

    /// Sessions that work in a repository, each in its own copy.
    mod code {
        use std::path::Path;

        use super::*;
        use crate::{Repository, SessionUpdate, Workspaces};

        struct Fixture {
            agent: LocalAgent<ScriptedRunner>,
            repository: Repository,
            _dir: tempfile::TempDir,
        }

        /// One turn per rule; turn `n` writes `a.txt` with `n`.
        fn fixture() -> Fixture {
            let dir = tempfile::tempdir().expect("a temporary folder");
            let path = dir.path().join("app");
            std::fs::create_dir_all(&path).expect("made the folder");
            std::fs::write(path.join("README.md"), "hello\n").expect("wrote");
            for arguments in [
                &["init", "--quiet", "--initial-branch=main"][..],
                &["add", "README.md"],
                &["commit", "--quiet", "-m", "first"],
            ] {
                let status = std::process::Command::new("git")
                    .arg("-C")
                    .arg(&path)
                    .args(arguments)
                    .env("GIT_AUTHOR_NAME", "Test")
                    .env("GIT_AUTHOR_EMAIL", "test@localhost")
                    .env("GIT_COMMITTER_NAME", "Test")
                    .env("GIT_COMMITTER_EMAIL", "test@localhost")
                    .status()
                    .expect("git runs");
                assert!(status.success(), "git {arguments:?}");
            }
            let rules = ["one", "two", "three"]
                .into_iter()
                .map(|turn| rule(turn, vec![write_file("a.txt", turn)]))
                .collect();
            let agent = LocalAgent::open(
                dir.path().join("state"),
                ScriptedRunner::new(Script { rules }),
            )
            .expect("the agent opens")
            .with_workspaces(Workspaces::new(dir.path().join("worktrees")));
            Fixture {
                agent,
                repository: Repository {
                    name: "app".into(),
                    path,
                },
                _dir: dir,
            }
        }

        impl Fixture {
            async fn start(&self) -> SessionId {
                let options = StartOptions {
                    name: "test".into(),
                    approval: ApprovalMode::AllowEverything,
                    repositories: vec![self.repository.clone()],
                    ..StartOptions::default()
                };
                self.agent.start(options).await.expect("starts").id
            }

            async fn turn(&self, id: &SessionId, text: &str, count: usize) -> Vec<Event> {
                self.agent
                    .send(id, text.into(), SendMode::Enqueue)
                    .await
                    .expect("sent");
                wait_for(&self.agent, id, idle_turns(count)).await
            }

            async fn file(&self, id: &SessionId) -> Option<String> {
                let record = record(&self.agent, id).await;
                read(&record.working_directory.join("a.txt"))
            }
        }

        async fn record(
            agent: &LocalAgent<ScriptedRunner>,
            id: &SessionId,
        ) -> crate::SessionRecord {
            let sessions = agent.sessions().await.expect("listed");
            sessions
                .into_iter()
                .find(|record| &record.id == id)
                .expect("the session exists")
        }

        fn read(path: &Path) -> Option<String> {
            std::fs::read_to_string(path).ok()
        }

        fn failed(events: &[Event]) -> Vec<String> {
            events
                .iter()
                .filter_map(|event| match &event.kind {
                    EventKind::Failed { message } => Some(message.clone()),
                    _ => None,
                })
                .collect()
        }

        #[tokio::test]
        async fn a_session_works_in_its_own_copy_of_the_repository() {
            let fixture = fixture();
            let id = fixture.start().await;
            let events = fixture.turn(&id, "one", 1).await;
            assert_eq!(failed(&events), Vec::<String>::new());
            assert_eq!(fixture.file(&id).await.as_deref(), Some("one"));
            assert_eq!(read(&fixture.repository.path.join("a.txt")), None);
        }

        #[tokio::test]
        async fn a_fork_has_the_code_and_history_as_they_were_where_it_starts() {
            let fixture = fixture();
            let id = fixture.start().await;
            fixture.turn(&id, "one", 1).await;
            let original = fixture.turn(&id, "two", 2).await;
            let second = user_message(&original, "two");

            let options = ForkOptions {
                before: Some(second.id.clone()),
                ..ForkOptions::default()
            };
            let fork = fixture.agent.fork(&id, options).await.expect("forked");
            assert_ne!(
                fork.working_directory,
                record(&fixture.agent, &id).await.working_directory
            );
            assert_eq!(fixture.file(&fork.id).await.as_deref(), Some("one"));
            assert_eq!(fixture.file(&id).await.as_deref(), Some("two"));
            let history = fixture.agent.history(&fork.id).await.expect("history");
            assert!(!history.iter().any(|event| event.id == second.id));

            fixture.turn(&fork.id, "three", 2).await;
            assert_eq!(fixture.file(&fork.id).await.as_deref(), Some("three"));
            assert_eq!(fixture.file(&id).await.as_deref(), Some("two"));
        }

        #[tokio::test]
        async fn a_rewind_cuts_the_chat_and_code_and_undoing_it_brings_both_back() {
            let fixture = fixture();
            let id = fixture.start().await;
            fixture.turn(&id, "one", 1).await;
            let original = fixture.turn(&id, "two", 2).await;
            let second = user_message(&original, "two");

            fixture
                .agent
                .rewind(&id, &second.id, true)
                .await
                .expect("rewound");
            assert_eq!(fixture.file(&id).await.as_deref(), Some("one"));
            let history = fixture.agent.history(&id).await.expect("history");
            assert!(!history.iter().any(|event| event.id == second.id));
            assert!(matches!(
                history.last().map(|event| &event.kind),
                Some(EventKind::Rewound { files: true, .. })
            ));

            fixture.agent.undo_rewind(&id).await.expect("undone");
            assert_eq!(fixture.file(&id).await.as_deref(), Some("two"));
            let history = fixture.agent.history(&id).await.expect("history");
            assert!(history.iter().any(|event| event.id == second.id));
            assert!(matches!(
                fixture.agent.undo_rewind(&id).await,
                Err(AgentError::NothingToUndo { .. })
            ));
        }

        #[tokio::test]
        async fn a_rewind_of_the_chat_only_leaves_the_code_and_the_session_carries_on() {
            let fixture = fixture();
            let id = fixture.start().await;
            fixture.turn(&id, "one", 1).await;
            let original = fixture.turn(&id, "two", 2).await;
            let second = user_message(&original, "two");

            fixture
                .agent
                .rewind(&id, &second.id, false)
                .await
                .expect("rewound");
            assert_eq!(fixture.file(&id).await.as_deref(), Some("two"));
            fixture.turn(&id, "three", 2).await;
            assert_eq!(fixture.file(&id).await.as_deref(), Some("three"));
            assert!(matches!(
                fixture.agent.undo_rewind(&id).await,
                Err(AgentError::NothingToUndo { .. })
            ));
        }

        #[tokio::test]
        async fn only_a_message_of_the_persons_can_be_rewound_to() {
            let fixture = fixture();
            let id = fixture.start().await;
            let events = fixture.turn(&id, "one", 1).await;
            let reply = events
                .iter()
                .find(|event| !matches!(event.kind, EventKind::UserMessage { .. }))
                .expect("an event that isn't a message");
            assert!(matches!(
                fixture.agent.rewind(&id, &reply.id, false).await,
                Err(AgentError::NotAMessage { .. })
            ));
        }

        #[tokio::test]
        async fn a_fork_before_the_first_message_starts_from_the_repository() {
            let fixture = fixture();
            let id = fixture.start().await;
            let events = fixture.turn(&id, "one", 1).await;
            let first = user_message(&events, "one");
            let options = ForkOptions {
                before: Some(first.id),
                ..ForkOptions::default()
            };
            let fork = fixture.agent.fork(&id, options).await.expect("forked");
            assert_eq!(fixture.file(&fork.id).await, None);
            assert!(fork.working_directory.join("README.md").is_file());
        }

        #[tokio::test]
        async fn a_fork_of_the_chat_only_works_in_its_parents_copy() {
            let fixture = fixture();
            let id = fixture.start().await;
            fixture.turn(&id, "one", 1).await;
            let options = ForkOptions {
                chat_only: true,
                ..ForkOptions::default()
            };
            let fork = fixture.agent.fork(&id, options).await.expect("forked");
            fixture.turn(&fork.id, "two", 2).await;
            assert_eq!(fixture.file(&id).await.as_deref(), Some("two"));
        }

        #[tokio::test]
        async fn a_restored_session_has_its_code_and_history_and_carries_on() {
            let fixture = fixture();
            let id = fixture.start().await;
            let history = fixture.turn(&id, "one", 1).await;
            let backup = fixture.agent.backup(&id).await.expect("backed up");

            let elsewhere = tempfile::tempdir().expect("a temporary folder");
            let rules = vec![rule("two", vec![write_file("a.txt", "two")])];
            let agent = LocalAgent::open(
                elsewhere.path().join("state"),
                ScriptedRunner::new(Script { rules }),
            )
            .expect("the agent opens")
            .with_workspaces(Workspaces::new(elsewhere.path().join("worktrees")));
            // Another host's clone, with the snapshots fetched into it.
            let clone = elsewhere.path().join("app");
            let original = fixture.repository.path.to_string_lossy().into_owned();
            let status = std::process::Command::new("git")
                .args(["clone", "--quiet", &original])
                .arg(&clone)
                .status()
                .expect("git runs");
            assert!(status.success());
            let status = std::process::Command::new("git")
                .arg("-C")
                .arg(&clone)
                .args(["fetch", "--quiet", &original, "refs/humata/*:refs/humata/*"])
                .status()
                .expect("git runs");
            assert!(status.success());
            let repositories = vec![Repository {
                name: "app".into(),
                path: clone,
            }];
            let restored = agent
                .restore(backup.clone(), repositories.clone())
                .await
                .expect("restored");
            assert_eq!(restored.id, id);
            assert!(restored.working_directory.starts_with(elsewhere.path()));
            assert_eq!(
                read(&restored.working_directory.join("a.txt")).as_deref(),
                Some("one")
            );
            let restored_history = agent.history(&id).await.expect("history");
            assert_eq!(restored_history[..history.len()], history[..]);
            assert!(matches!(
                agent.restore(backup, repositories).await,
                Err(AgentError::SessionExists { .. })
            ));

            agent
                .send(&id, "two".into(), SendMode::Enqueue)
                .await
                .expect("sent");
            let events = wait_for(&agent, &id, idle_turns(2)).await;
            assert_eq!(failed(&events), Vec::<String>::new());
            assert_eq!(
                read(&restored.working_directory.join("a.txt")).as_deref(),
                Some("two")
            );
        }

        #[tokio::test]
        async fn a_restore_that_fails_part_way_leaves_nothing_and_can_be_tried_again() {
            let fixture = fixture();
            let id = fixture.start().await;
            fixture.turn(&id, "one", 1).await;
            let backup = fixture.agent.backup(&id).await.expect("backed up");
            let elsewhere = tempfile::tempdir().expect("a temporary folder");
            let agent = LocalAgent::open(
                elsewhere.path().join("state"),
                ScriptedRunner::new(Script { rules: Vec::new() }),
            )
            .expect("the agent opens")
            .with_workspaces(Workspaces::new(elsewhere.path().join("worktrees")));
            let clone = elsewhere.path().join("app");
            let original = fixture.repository.path.to_string_lossy().into_owned();
            let git = |arguments: &[&str]| {
                let status = std::process::Command::new("git")
                    .args(arguments)
                    .status()
                    .expect("git runs");
                assert!(status.success(), "git {arguments:?}");
            };
            let clone_path = clone.to_string_lossy().into_owned();
            // Not a local clone, which would link in every object, the
            // snapshots' commits too.
            git(&["clone", "--quiet", "--no-local", &original, &clone_path]);
            let repositories = vec![Repository {
                name: "app".into(),
                path: clone,
            }];

            // The clone lacks the session's snapshots, so its code can't be
            // made again.
            assert!(
                agent
                    .restore(backup.clone(), repositories.clone())
                    .await
                    .is_err()
            );
            assert!(agent.sessions().await.expect("listed").is_empty());

            git(&[
                "-C",
                &clone_path,
                "fetch",
                "--quiet",
                &original,
                "refs/humata/*:refs/humata/*",
            ]);
            let restored = agent
                .restore(backup, repositories)
                .await
                .expect("restored on the second try");
            assert_eq!(
                read(&restored.working_directory.join("a.txt")).as_deref(),
                Some("one")
            );
        }

        #[tokio::test]
        async fn a_restore_without_a_repository_the_session_works_in_is_refused() {
            let fixture = fixture();
            let id = fixture.start().await;
            fixture.turn(&id, "one", 1).await;
            let backup = fixture.agent.backup(&id).await.expect("backed up");
            let elsewhere = tempfile::tempdir().expect("a temporary folder");
            let agent = LocalAgent::open(
                elsewhere.path().join("state"),
                ScriptedRunner::new(Script { rules: Vec::new() }),
            )
            .expect("the agent opens")
            .with_workspaces(Workspaces::new(elsewhere.path().join("worktrees")));
            assert!(matches!(
                agent.restore(backup, Vec::new()).await,
                Err(AgentError::MissingRepository { .. })
            ));
        }

        #[tokio::test]
        async fn an_archived_session_takes_no_messages_and_gets_its_code_back_when_unarchived() {
            let fixture = fixture();
            let id = fixture.start().await;
            fixture.turn(&id, "one", 1).await;
            let folder = record(&fixture.agent, &id).await.working_directory;

            let archive = SessionUpdate {
                archived: Some(true),
                ..SessionUpdate::default()
            };
            let record = fixture.agent.update(&id, archive).await.expect("archived");
            assert!(record.archived);
            assert!(!folder.exists());
            let refused = fixture
                .agent
                .send(&id, "two".into(), SendMode::Enqueue)
                .await;
            assert!(
                matches!(refused, Err(AgentError::Archived { .. })),
                "{refused:?}"
            );
            let changes = fixture
                .agent
                .changes(&id, &crate::Query::default())
                .await
                .expect("compared");
            assert!(changes[0].patch.contains("+one"), "{changes:?}");

            let unarchive = SessionUpdate {
                archived: Some(false),
                ..SessionUpdate::default()
            };
            fixture
                .agent
                .update(&id, unarchive)
                .await
                .expect("unarchived");
            let events = fixture.turn(&id, "two", 2).await;
            assert_eq!(failed(&events), Vec::<String>::new());
            let changes = fixture
                .agent
                .changes(&id, &crate::Query::default())
                .await
                .expect("compared");
            assert!(changes[0].patch.contains("+two"), "{changes:?}");
        }

        #[tokio::test]
        async fn archiving_again_puts_away_what_a_failed_archive_left() {
            let fixture = fixture();
            let id = fixture.start().await;
            fixture.turn(&id, "one", 1).await;
            let folder = record(&fixture.agent, &id).await.working_directory;
            // The repository goes missing, so the archive can't finish.
            let repository = fixture.repository.path.clone();
            let moved = repository.with_extension("away");
            std::fs::rename(&repository, &moved).expect("moved the repository");
            let archive = SessionUpdate {
                archived: Some(true),
                ..SessionUpdate::default()
            };
            let failed = fixture.agent.update(&id, archive.clone()).await;
            assert!(failed.is_err(), "{failed:?}");
            std::fs::rename(&moved, &repository).expect("moved the repository back");

            let record = fixture.agent.update(&id, archive).await.expect("archived");
            assert!(record.archived);
            assert!(!folder.exists());
        }

        #[tokio::test]
        async fn renaming_and_marking_the_keeper_are_kept() {
            let fixture = fixture();
            let id = fixture.start().await;
            let update = SessionUpdate {
                name: Some("better".into()),
                keeper: Some(true),
                ..SessionUpdate::default()
            };
            fixture.agent.update(&id, update).await.expect("updated");
            let record = record(&fixture.agent, &id).await;
            assert_eq!(record.name, "better");
            assert!(record.keeper);
        }

        #[tokio::test]
        async fn a_session_in_a_repository_needs_somewhere_for_its_copy() {
            let dir = tempfile::tempdir().expect("a temporary folder");
            let agent = LocalAgent::open(dir.path(), ScriptedRunner::new(Script::default()))
                .expect("the agent opens");
            let options = StartOptions {
                repositories: vec![Repository {
                    name: "app".into(),
                    path: dir.path().join("app"),
                }],
                ..StartOptions::default()
            };
            let result = agent.start(options).await;
            assert!(
                matches!(result, Err(AgentError::NoWorkspaces { .. })),
                "{result:?}"
            );
        }
    }
}
