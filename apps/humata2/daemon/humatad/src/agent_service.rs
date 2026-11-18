//! AgentService (settings.proto; ST-1, ST-2): the agent kinds this daemon
//! can start, with what each supports, read from the `Agents` it runs
//! sessions with.
//!
//! The kinds come from the daemon's agents file (`--agents`, a JSON list)
//! or, without one, the built-in list, plus the plain shell a Terminal
//! session runs. They are read once at start, so adding or editing a kind
//! is a change to that file and a daemon restart; AddAgent and UpdateAgent
//! say so and change nothing.
//!
//! Left empty in each AgentDefinition, since the daemon doesn't know them:
//! `variables`, `fork_arguments`, `models` and `revision`.

use humata2_grpc::humata::v1 as pb;
use humata2_sessions::{AgentKind, Agents, TurnEnd};
use tonic::{Code, Request, Response, Status};

use crate::agent_switches::{self, AgentSwitches, Change, SwitchesError};
use crate::identity;
use crate::{agent_gate, status};
use pb::invalid_field::Problem;

/// The agents the daemon runs, and whether they are its built-in list.
pub(crate) struct AgentSvc {
    agents: Agents,
    /// Whether no agents file was given, so the kinds are the built-in ones.
    built_in: bool,
}

impl AgentSvc {
    pub(crate) fn new(agents: Agents, built_in: bool) -> Self {
        Self { agents, built_in }
    }

    /// The switches in the file now, read afresh as every check does; none
    /// when the daemon has no home to keep them in, which means the defaults.
    fn switches(&self) -> Option<AgentSwitches> {
        agent_gate::switches(&self.agents)
    }

    fn definition(&self, kind: &AgentKind, switches: Option<&AgentSwitches>) -> pb::AgentDefinition {
        let mut launch = vec![kind.program.to_string_lossy().into_owned()];
        launch.extend(kind.start_args.iter().cloned());
        pb::AgentDefinition {
            id: Some(pb::AgentId {
                value: kind.name.clone(),
            }),
            name: self.agents.person_name(&kind.name),
            // The plain shell is the daemon's own, whatever the file says.
            built_in: self.built_in || kind.turn_end == TurnEnd::None && kind.name == "zsh",
            launch_command: launch.join(" "),
            variables: Vec::new(),
            resume_arguments: kind.resume_args.join(" "),
            fork_arguments: String::new(),
            supports_context_fork: kind.conversation_format
                != humata2_sessions::ConversationFormat::None,
            supports_resume: !kind.resume_args.is_empty() && !kind.conversation_file.is_empty(),
            supports_turn_signal: kind.turn_end != TurnEnd::None,
            models: Vec::new(),
            revision: None,
            turn_end_source: match kind.turn_end {
                TurnEnd::Hooks => pb::TurnEndSource::Hooks,
                TurnEnd::Output => pb::TurnEndSource::OutputFallback,
                TurnEnd::None => pb::TurnEndSource::None,
            } as i32,
            enabled: switches.is_none_or(|switches| switches.enabled(&kind.name)),
            install_when_missing: switches.is_some_and(|switches| switches.install_when_missing(&kind.name)),
            can_install: can_install(switches, &kind.name),
            install_command: switches
                .and_then(|switches| switches.install_command(&kind.name))
                .or_else(|| agent_switches::builtin_install_command(&kind.name).map(str::to_owned))
                .unwrap_or_default(),
        }
    }
}

/// AddAgent and UpdateAgent: the agents are the daemon's file's, not
/// settings it can change while running.
fn set_in_file(method: &str) -> Status {
    status::failure(
        Code::FailedPrecondition,
        pb::Error {
            message: format!("{method}: this daemon takes its agents from its agents file, not from a call"),
            fix: "edit the agents file given to humatad2's --agents (a JSON list of agents with name, program, start_args, resume_args and conversation_file), then restart the daemon; running sessions keep their commands"
                .to_owned(),
            retry: None,
            kind: None,
        },
    )
}

/// Whether Humata has a command to install `kind`: the file's, or the
/// compiled-in one when there is no file.
fn can_install(switches: Option<&AgentSwitches>, kind: &str) -> bool {
    match switches {
        Some(switches) => switches.install_command(kind).is_some(),
        None => agent_switches::builtin_install_command(kind).is_some(),
    }
}

/// A switches file that couldn't be read or written.
fn switches_failed(error: &SwitchesError) -> Status {
    status::failure(
        Code::FailedPrecondition,
        pb::Error {
            message: status::chain(error),
            fix: "fix or remove ~/.humata2/config/agents.toml, then change the switch again".to_owned(),
            retry: None,
            kind: None,
        },
    )
}

#[tonic::async_trait]
impl pb::agent_service_server::AgentService for AgentSvc {
    async fn list_agents(
        &self,
        request: Request<pb::ListAgentsRequest>,
    ) -> Result<Response<pb::ListAgentsResponse>, Status> {
        let all = request.into_inner().include_disabled;
        let switches = self.switches();
        Ok(Response::new(pb::ListAgentsResponse {
            agents: self
                .agents
                .kinds()
                .iter()
                .filter(|kind| all || switches.as_ref().is_none_or(|switches| switches.enabled(&kind.name)))
                .map(|kind| self.definition(kind, switches.as_ref()))
                .collect(),
        }))
    }

    async fn set_agent_enabled(
        &self,
        request: Request<pb::SetAgentEnabledRequest>,
    ) -> Result<Response<pb::AgentDefinition>, Status> {
        crate::desk_service::user_only(&identity::caller(&request)?)?;
        let request = request.into_inner();
        let kind = request
            .agent
            .map(|agent| agent.value)
            .filter(|agent| !agent.is_empty())
            .ok_or_else(|| status::missing("agent"))?;
        let found = self.agents.get(&kind).map_err(|error| {
            status::invalid_field(
                "agent",
                Problem::UnknownValue,
                status::chain(&error),
                "pick one of the agents Settings › Agents lists",
            )
        })?;
        if kind == "zsh" {
            return Err(status::invalid_field(
                "agent",
                Problem::NotAllowedHere,
                "the plain shell is not an agent, so it has no switches",
                "leave Terminal as it is; the switches are for the agents",
            ));
        }
        let current = self.switches();
        if request.install_when_missing == Some(true) && !can_install(current.as_ref(), &kind) {
            return Err(status::invalid_field(
                "install_when_missing",
                Problem::NotAllowedHere,
                format!("Humata has no command to install {}", self.agents.person_name(&found.name)),
                "install it on each machine yourself, or give the agent an `install` command in agents.toml first",
            ));
        }
        let Some(path) = self.agents.settings_file() else {
            return Err(status::failure(
                Code::FailedPrecondition,
                pb::Error {
                    message: "this daemon was started without a home folder, so it has nowhere to keep the agent switches".to_owned(),
                    fix: "start humatad2 with a HOME (or --root) set, then change the switch again".to_owned(),
                    retry: None,
                    kind: None,
                },
            ));
        };
        let path = path.to_owned();
        let change = Change {
            enabled: request.enabled,
            install_when_missing: request.install_when_missing,
        };
        let name = kind.clone();
        let written = tokio::task::spawn_blocking(move || agent_switches::set(&path, &name, change))
            .await
            .map_err(|error| status::internal(format!("saving the agent switches stopped: {error}"), ""))?
            .map_err(|error| switches_failed(&error))?;
        let found = self.agents.get(&kind).map_err(|error| {
            status::internal(format!("the agent {kind} went missing: {}", status::chain(&error)), "")
        })?;
        Ok(Response::new(self.definition(found, Some(&written))))
    }

    async fn add_agent(
        &self,
        _request: Request<pb::AddAgentRequest>,
    ) -> Result<Response<pb::AgentDefinition>, Status> {
        Err(set_in_file("AddAgent"))
    }

    async fn update_agent(
        &self,
        _request: Request<pb::UpdateAgentRequest>,
    ) -> Result<Response<pb::AgentDefinition>, Status> {
        Err(set_in_file("UpdateAgent"))
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::Path;

    use pb::agent_service_server::AgentService;

    use super::*;
    use crate::identity::Caller;

    fn service() -> AgentSvc {
        let agents = Agents::builtin().with_shell(Path::new("/bin/zsh"), &[OsString::from("-l")]);
        AgentSvc::new(agents, true)
    }

    #[tokio::test]
    async fn the_listed_agents_say_what_each_kind_supports() {
        let listed = service()
            .list_agents(Request::new(pb::ListAgentsRequest::default()))
            .await
            .unwrap()
            .into_inner()
            .agents;

        let id = |agent: &pb::AgentDefinition| agent.id.as_ref().map(|id| id.value.clone());
        assert_eq!(id(&listed[0]).as_deref(), Some("claude"));
        assert!(listed.iter().all(|agent| agent.built_in));
        let claude = &listed[0];
        assert!(claude.supports_resume && claude.supports_turn_signal && claude.supports_context_fork);
        assert_eq!(claude.turn_end_source, pb::TurnEndSource::Hooks as i32);
        let shell = listed
            .iter()
            .find(|agent| id(agent).as_deref() == Some("zsh"))
            .unwrap();
        assert!(!shell.supports_resume && !shell.supports_turn_signal && !shell.supports_context_fork);
        assert_eq!(shell.turn_end_source, pb::TurnEndSource::None as i32);
        assert_eq!(shell.launch_command, "/bin/zsh -l");
    }

    #[tokio::test]
    async fn adding_or_updating_an_agent_is_refused() {
        let svc = service();

        let added = svc
            .add_agent(Request::new(pb::AddAgentRequest::default()))
            .await
            .unwrap_err();
        let updated = svc
            .update_agent(Request::new(pb::UpdateAgentRequest::default()))
            .await
            .unwrap_err();

        assert_eq!(added.code(), Code::FailedPrecondition);
        assert_eq!(updated.code(), Code::FailedPrecondition);
    }

    fn with_switches(toml: &str) -> (AgentSvc, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("agents.toml");
        std::fs::write(&file, toml).unwrap();
        let agents = Agents::builtin()
            .with_shell(Path::new("/bin/zsh"), &[OsString::from("-l")])
            .with_settings(file);
        (AgentSvc::new(agents, true), dir)
    }

    fn ids(listed: &[pb::AgentDefinition]) -> Vec<String> {
        listed
            .iter()
            .filter_map(|agent| agent.id.as_ref().map(|id| id.value.clone()))
            .collect()
    }

    fn set(agent: &str, enabled: Option<bool>, install: Option<bool>, caller: Caller) -> Request<pb::SetAgentEnabledRequest> {
        let mut request = Request::new(pb::SetAgentEnabledRequest {
            agent: Some(pb::AgentId { value: agent.to_owned() }),
            enabled,
            install_when_missing: install,
        });
        request.extensions_mut().insert(caller);
        request
    }

    fn user() -> Caller {
        Caller::User(identity::UserVia::Host)
    }

    #[tokio::test]
    async fn a_disabled_kind_is_listed_only_when_asked_for() {
        let (svc, _dir) = with_switches("[agents.copilot]\nenabled = false\n");

        let shown = svc
            .list_agents(Request::new(pb::ListAgentsRequest::default()))
            .await
            .unwrap()
            .into_inner()
            .agents;
        let all = svc
            .list_agents(Request::new(pb::ListAgentsRequest { include_disabled: true }))
            .await
            .unwrap()
            .into_inner()
            .agents;

        assert!(!ids(&shown).contains(&"copilot".to_owned()));
        assert!(ids(&shown).contains(&"zsh".to_owned()));
        let copilot = all.iter().find(|agent| ids(std::slice::from_ref(agent)) == ["copilot"]).unwrap();
        assert!(!copilot.enabled);
    }

    #[tokio::test]
    async fn set_agent_enabled_writes_the_switch_and_answers_the_definition() {
        let (svc, _dir) = with_switches("");

        let off = svc
            .set_agent_enabled(set("codex", Some(false), None, user()))
            .await
            .unwrap()
            .into_inner();
        let on = svc
            .set_agent_enabled(set("codex", None, Some(true), user()))
            .await
            .unwrap()
            .into_inner();

        assert!(!off.enabled && !off.install_when_missing && off.can_install);
        assert!(!on.enabled && on.install_when_missing);
        let listed = svc
            .list_agents(Request::new(pb::ListAgentsRequest::default()))
            .await
            .unwrap()
            .into_inner()
            .agents;
        assert!(!ids(&listed).contains(&"codex".to_owned()));
    }

    #[tokio::test]
    async fn set_agent_enabled_refuses_what_it_cannot_do() {
        let (svc, _dir) = with_switches("");

        let by_peer = svc.set_agent_enabled(set("codex", Some(false), None, Caller::Peer)).await.unwrap_err();
        let unknown = svc.set_agent_enabled(set("nope", Some(false), None, user())).await.unwrap_err();
        let shell = svc.set_agent_enabled(set("zsh", Some(false), None, user())).await.unwrap_err();

        assert_ne!(by_peer.code(), Code::Ok);
        assert_eq!(unknown.code(), Code::InvalidArgument);
        assert_eq!(shell.code(), Code::InvalidArgument);
    }

    #[tokio::test]
    async fn set_agent_enabled_without_a_home_is_a_failed_precondition() {
        let svc = service();

        let refused = svc
            .set_agent_enabled(set("codex", Some(false), None, user()))
            .await
            .unwrap_err();

        assert_eq!(refused.code(), Code::FailedPrecondition);
    }
}
