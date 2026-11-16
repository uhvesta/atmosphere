use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::AgentError;

/// How to start and resume one kind of agent. In `start_args`,
/// `resume_args` and `conversation_file`, `{conversation}` is the pinned
/// conversation id (PER-12), `{folder}` the session folder, `{folder_dashed}`
/// the folder with `/` and `.` replaced by `-`, and `{home}` the home folder.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct AgentKind {
    pub name: String,
    pub program: PathBuf,
    pub start_args: Vec<String>,
    pub resume_args: Vec<String>,
    pub conversation_file: String,
    /// Where this agent's turn ends come from (SE-21, D169).
    #[serde(default)]
    pub turn_end: TurnEnd,
    /// What the agent's conversation file holds, so its turns can be read
    /// for a fork before one (FK-3). None unless the kind says otherwise.
    #[serde(default)]
    pub conversation_format: ConversationFormat,
}

/// The format of an agent's conversation file.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ConversationFormat {
    /// Not read: the agent has no turns to fork before, only forks from now.
    #[default]
    None,
    /// Claude's JSONL, one JSON object per line, whose turns are read by
    /// `humata2_mirror::turns_of`.
    ClaudeJsonl,
}

impl ConversationFormat {
    /// Whether the turns of a conversation in this format can be read.
    pub fn reads_turns(self) -> bool {
        self == Self::ClaudeJsonl
    }
}

/// Where an agent's turn ends are learned from.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TurnEnd {
    /// The agent's own hooks (`{hooks}` in its args is the hooks file).
    Hooks,
    /// Output that has gone quiet (SE-22).
    #[default]
    Output,
    /// A plain program with no turns.
    None,
}

/// What `AgentKind::launch` fills in; maps onto `humata2_store::AgentLaunch`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchPlan {
    pub argv: Vec<OsString>,
    pub resume_argv: Vec<OsString>,
    pub conversation_file: PathBuf,
}

impl AgentKind {
    /// The commands and conversation file for `conversation` in `folder`.
    pub fn launch(&self, home: &Path, folder: &Path, conversation: &str) -> LaunchPlan {
        let folder_text = folder.to_string_lossy();
        let fill = |t: &str| {
            t.replace("{conversation}", conversation)
                .replace("{folder_dashed}", &dashed(&folder_text))
                .replace("{folder}", &folder_text)
                .replace("{home}", &home.to_string_lossy())
        };
        let argv = |args: &[String]| {
            std::iter::once(self.program.clone().into_os_string())
                .chain(args.iter().map(|a| OsString::from(fill(a))))
                .collect()
        };
        LaunchPlan {
            argv: argv(&self.start_args),
            resume_argv: argv(&self.resume_args),
            conversation_file: PathBuf::from(fill(&self.conversation_file)),
        }
    }
}

/// OpenCode, run by the `humata opencode` wrapper (D201, D201a), which
/// starts its server, attaches its TUI to the conversation's session, and
/// reports turns and asks as the session's agent. It starts and resumes the
/// same way: the session is named after the conversation. Its conversation
/// lives in OpenCode's own database, so there is no file to read.
fn opencode() -> AgentKind {
    let args: Vec<String> = [
        "HUMATA_SOCKET={humata_socket}",
        "{humata}",
        "opencode",
        "--conversation",
        "{conversation}",
        "--state",
        "{state}/opencode/{conversation}.json",
    ]
    .map(String::from)
    .into();
    AgentKind {
        name: "opencode".into(),
        // Sets the socket the wrapper talks to, with no shell between.
        program: "/usr/bin/env".into(),
        start_args: args.clone(),
        resume_args: args,
        conversation_file: String::new(),
        turn_end: TurnEnd::Hooks,
        conversation_format: ConversationFormat::None,
    }
}

fn dashed(folder: &str) -> String {
    folder.replace(['/', '.'], "-")
}

/// The agents a session can run.
#[derive(Clone, Debug)]
pub struct Agents {
    kinds: Vec<AgentKind>,
}

impl Agents {
    pub fn builtin() -> Self {
        Self {
            kinds: vec![
                AgentKind {
                    name: "claude".into(),
                    program: "claude".into(),
                    start_args: vec![
                        "--session-id".into(),
                        "{conversation}".into(),
                        "--settings".into(),
                        "{hooks}".into(),
                    ],
                    resume_args: vec![
                        "--resume".into(),
                        "{conversation}".into(),
                        "--settings".into(),
                        "{hooks}".into(),
                    ],
                    conversation_file:
                        "{home}/.claude/projects/{folder_dashed}/{conversation}.jsonl".into(),
                    turn_end: TurnEnd::Hooks,
                    conversation_format: ConversationFormat::ClaudeJsonl,
                },
                opencode(),
            ],
        }
    }

    /// The kinds, in order.
    pub fn kinds(&self) -> &[AgentKind] {
        &self.kinds
    }

    /// Every `{hooks}` in a kind's `start_args` and `resume_args` becomes
    /// `file`, the settings file with the agent's turn hooks (D169).
    pub fn with_hooks(mut self, file: &Path) -> Self {
        let file = file.to_string_lossy();
        for kind in &mut self.kinds {
            for arg in kind.start_args.iter_mut().chain(&mut kind.resume_args) {
                *arg = arg.replace("{hooks}", &file);
            }
        }
        self
    }

    /// Every `{codex_hooks}` in a kind's args becomes `codex`, the value of
    /// Codex's `-c hooks=…`, and every `{copilot_plugin}` the folder of the
    /// plugin with Copilot's hooks (HOOKS, D193). Each is one argument, so
    /// no shell ever splits it.
    pub fn with_agent_hooks(mut self, codex: &str, copilot_plugin: &Path) -> Self {
        let plugin = copilot_plugin.to_string_lossy();
        for kind in &mut self.kinds {
            for arg in kind.start_args.iter_mut().chain(&mut kind.resume_args) {
                *arg = arg
                    .replace("{codex_hooks}", codex)
                    .replace("{copilot_plugin}", &plugin);
            }
        }
        self
    }

    /// Every `{humata}` in a kind's args becomes `humata`, the CLI beside
    /// the daemon, every `{humata_socket}` the daemon's socket, and every
    /// `{state}` its state folder: what the `humata opencode` wrapper is run
    /// with (D201a).
    pub fn with_wrapper(mut self, humata: &Path, socket: &Path, state: &Path) -> Self {
        let (humata, socket, state) = (
            humata.to_string_lossy(),
            socket.to_string_lossy(),
            state.to_string_lossy(),
        );
        for kind in &mut self.kinds {
            for arg in kind.start_args.iter_mut().chain(&mut kind.resume_args) {
                *arg = arg
                    .replace("{humata_socket}", &socket)
                    .replace("{humata}", &humata)
                    .replace("{state}", &state);
            }
        }
        self
    }

    /// A JSON list of kinds that replaces the built-ins.
    pub fn from_json(bytes: &[u8]) -> Result<Self, AgentError> {
        let kinds: Vec<AgentKind> = serde_json::from_slice(bytes).map_err(AgentError::Parse)?;
        for (i, k) in kinds.iter().enumerate() {
            if kinds[..i].iter().any(|o| o.name == k.name) {
                return Err(AgentError::Duplicate {
                    name: k.name.clone(),
                });
            }
        }
        Ok(Self { kinds })
    }

    pub fn get(&self, name: &str) -> Result<&AgentKind, AgentError> {
        self.kinds
            .iter()
            .find(|k| k.name == name)
            .ok_or_else(|| AgentError::Unknown {
                name: name.to_string(),
                known: self
                    .kinds
                    .iter()
                    .map(|k| k.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_pins_the_conversation() {
        let agents = Agents::builtin().with_hooks(Path::new("/state/claude-hooks.json"));
        let plan = agents.get("claude").unwrap().launch(
            Path::new("/home/me"),
            Path::new("/home/me/Humata/ch/sessions/s.1"),
            "abc",
        );
        assert_eq!(
            plan.argv,
            [
                "claude",
                "--session-id",
                "abc",
                "--settings",
                "/state/claude-hooks.json"
            ]
        );
        assert_eq!(
            plan.resume_argv,
            [
                "claude",
                "--resume",
                "abc",
                "--settings",
                "/state/claude-hooks.json"
            ]
        );
        assert_eq!(agents.get("claude").unwrap().turn_end, TurnEnd::Hooks);
        assert_eq!(
            plan.conversation_file,
            Path::new("/home/me/.claude/projects/-home-me-Humata-ch-sessions-s-1/abc.jsonl")
        );
    }

    #[test]
    fn opencode_runs_through_humatas_wrapper_on_this_daemon() {
        let agents = Agents::builtin().with_wrapper(
            Path::new("/opt/humata/humata"),
            Path::new("/state dir/humatad.sock"),
            Path::new("/state dir"),
        );
        let kind = agents.get("opencode").expect("OpenCode is built in");
        assert_eq!(kind.turn_end, TurnEnd::Hooks);
        let plan = kind.launch(Path::new("/home/me"), Path::new("/work"), "abc");
        assert_eq!(
            plan.argv,
            [
                "/usr/bin/env",
                "HUMATA_SOCKET=/state dir/humatad.sock",
                "/opt/humata/humata",
                "opencode",
                "--conversation",
                "abc",
                "--state",
                "/state dir/opencode/abc.json",
            ]
        );
        // Resuming names the same session.
        assert_eq!(plan.resume_argv, plan.argv);
        assert!(!kind.conversation_format.reads_turns());
    }

    #[test]
    fn unknown_agent_is_an_error() {
        assert!(matches!(
            Agents::builtin().get("nope"),
            Err(AgentError::Unknown { .. })
        ));
    }

    #[test]
    fn json_replaces_builtins() {
        let json =
            br#"[{"name":"fake","program":"/bin/fake","start_args":["{folder}","{conversation}"],
            "resume_args":[],"conversation_file":"{folder}/c-{conversation}"}]"#;
        let agents = Agents::from_json(json).unwrap();
        assert!(agents.get("claude").is_err());
        let plan = agents
            .get("fake")
            .unwrap()
            .launch(Path::new("/h"), Path::new("/f"), "id");
        assert_eq!(plan.argv, ["/bin/fake", "/f", "id"]);
        assert_eq!(plan.resume_argv, ["/bin/fake"]);
        assert_eq!(plan.conversation_file, Path::new("/f/c-id"));
    }

    #[test]
    fn turn_end_defaults_to_output_and_is_read_from_json() {
        let json = br#"[{"name":"a","program":"p","start_args":["{hooks}"],"resume_args":[],
            "conversation_file":"x","turn_end":"hooks"},
            {"name":"b","program":"p","start_args":[],"resume_args":[],"conversation_file":"x"}]"#;
        let agents = Agents::from_json(json)
            .unwrap()
            .with_hooks(Path::new("/h.json"));
        assert_eq!(agents.get("a").unwrap().turn_end, TurnEnd::Hooks);
        assert_eq!(agents.get("a").unwrap().start_args, ["/h.json"]);
        assert_eq!(agents.get("b").unwrap().turn_end, TurnEnd::Output);
    }

    #[test]
    fn codex_and_copilot_hooks_fill_their_placeholders() {
        let json =
            br#"[{"name":"codex","program":"codex","start_args":["-c","hooks={codex_hooks}"],
            "resume_args":["resume","-c","hooks={codex_hooks}"],"conversation_file":"x"},
            {"name":"copilot","program":"copilot","start_args":["--plugin-dir","{copilot_plugin}"],
            "resume_args":[],"conversation_file":"x"}]"#;
        let agents = Agents::from_json(json)
            .unwrap()
            .with_agent_hooks("{ Stop = [] }", Path::new("/state dir/copilot-plugin"));
        let codex = agents.get("codex").unwrap();
        assert_eq!(codex.start_args, ["-c", "hooks={ Stop = [] }"]);
        assert_eq!(codex.resume_args, ["resume", "-c", "hooks={ Stop = [] }"]);
        assert_eq!(
            agents.get("copilot").unwrap().start_args,
            ["--plugin-dir", "/state dir/copilot-plugin"]
        );
    }

    #[test]
    fn conversation_format_is_read_from_json_and_is_none_by_default() {
        let json = br#"[{"name":"a","program":"p","start_args":[],"resume_args":[],
            "conversation_file":"x","conversation_format":"claude-jsonl"},
            {"name":"b","program":"p","start_args":[],"resume_args":[],"conversation_file":"x"}]"#;
        let agents = Agents::from_json(json).unwrap();
        assert!(agents.get("a").unwrap().conversation_format.reads_turns());
        assert!(!agents.get("b").unwrap().conversation_format.reads_turns());
        assert!(
            Agents::builtin()
                .get("claude")
                .unwrap()
                .conversation_format
                .reads_turns()
        );
    }

    #[test]
    fn bad_or_duplicate_json_errors() {
        assert!(Agents::from_json(b"nope").is_err());
        let k = r#"{"name":"a","program":"p","start_args":[],"resume_args":[],"conversation_file":"x"}"#;
        assert!(Agents::from_json(format!("[{k},{k}]").as_bytes()).is_err());
    }
}
