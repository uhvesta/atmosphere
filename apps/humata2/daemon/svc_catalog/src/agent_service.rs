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

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use humata2_daemon_ports::Caller;
use humata2_daemon_runtime::agent_switches::{
    self, AgentSwitches, Change, OnChange, SwitchesError,
};
use crate::pb;
use humata2_rpc_kit::{chain, failure, internal, invalid_field, missing};
use humata2_sessions_core::{AgentKind, Agents, ConversationFormat, TurnEnd};
use prost::Message;
use tonic::{Code, Request, Response, Status};

use crate::agent_gate;
use pb::invalid_field::Problem;

/// Boxed future returned by [`AgentSettingsGate`] closures.
pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// Narrow port letting `AgentSvc` route agent settings mutations through
/// `Capability::SettingsChange` Inbox gating (`AskNoAllow`) without pulling
/// `humatad_core` into `svc_catalog`.
#[tonic::async_trait]
pub trait AgentSettingsGate: Send + Sync {
    async fn gate_settings_change(
        &self,
        caller: &Caller,
        key: Option<String>,
        scope: String,
        changes: Vec<(String, String, String)>,
        reason: String,
        type_name: &'static str,
        run: Box<dyn FnOnce() -> BoxFuture<Result<Vec<u8>, Status>> + Send>,
    ) -> Result<Vec<u8>, Status>;
}

/// The agents the daemon runs, and whether they are its built-in list.
#[derive(Clone)]
pub struct AgentSvc {
    agents: Agents,
    /// Whether no agents file was given, so the kinds are the built-in ones.
    built_in: bool,
    /// Told after every switch it writes, so linked machines get the file.
    on_change: OnChange,
    /// Optional `SettingsChange` Inbox gate wired by `humatad_plugin_catalog`.
    gate: Option<Arc<dyn AgentSettingsGate>>,
}

impl AgentSvc {
    pub fn new(agents: Agents, built_in: bool) -> Self {
        Self {
            agents,
            built_in,
            on_change: OnChange::default(),
            gate: None,
        }
    }

    /// Routes agent mutations through `gate` (`Capability::SettingsChange`
    /// Inbox gating) instead of hard `UserOnly` refusal.
    pub fn with_gate(mut self, gate: Arc<dyn AgentSettingsGate>) -> Self {
        self.gate = Some(gate);
        self
    }

    /// The hook told after every switch this service writes.
    pub fn on_change(&self) -> OnChange {
        self.on_change.clone()
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
            supports_context_fork: kind.conversation_format != ConversationFormat::None,
            supports_resume: !kind.is_shell() && !kind.resume_args.is_empty(),
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
            asks_in_inbox: switches.is_none_or(|switches| switches.asks_in_inbox(&kind.name)),
            can_install: can_install(switches, &kind.name),
            install_command: switches
                .and_then(|switches| switches.install_command(&kind.name))
                .or_else(|| agent_switches::builtin_install_command(&kind.name).map(str::to_owned))
                .unwrap_or_default(),
        }
    }
}

fn caller<T>(request: &Request<T>) -> Result<Caller, Status> {
    request.extensions().get::<Caller>().cloned().ok_or_else(|| {
        failure(
            Code::Unauthenticated,
            pb::Error {
                message: "this call has no caller identity".to_owned(),
                fix: "sign in with `humata login` or call from the Humata app".to_owned(),
                retry: None,
                kind: None,
            },
        )
    })
}

fn user_only(caller: &Caller) -> Result<(), Status> {
    if matches!(caller, Caller::User(_)) {
        return Ok(());
    }
    Err(failure(
        Code::PermissionDenied,
        pb::Error {
            message: "only the user can change agent settings".to_owned(),
            fix: "change this setting in the Humata app or from the CLI as the user".to_owned(),
            retry: None,
            kind: Some(pb::error::Kind::UserOnly(pb::UserOnly::default())),
        },
    ))
}

/// AddAgent and UpdateAgent: the agents are the daemon's file's, not
/// settings it can change while running.
fn set_in_file(method: &str) -> Status {
    failure(
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
    failure(
        Code::FailedPrecondition,
        pb::Error {
            message: chain(error),
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
        let switches = agent_gate::read_switches(&self.agents).map_err(Status::from)?;
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
        let who = caller(&request)?;
        if self.gate.is_none() {
            user_only(&who)?;
        }
        let request = request.into_inner();
        let kind = request
            .agent
            .map(|agent| agent.value)
            .filter(|agent| !agent.is_empty())
            .ok_or_else(|| missing("agent"))?;
        let found = self.agents.get(&kind).map_err(|error| {
            invalid_field(
                "agent",
                Problem::UnknownValue,
                chain(&error),
                "pick one of the agents Settings › Agents lists",
            )
        })?;
        if kind == "zsh" {
            return Err(invalid_field(
                "agent",
                Problem::NotAllowedHere,
                "the plain shell is not an agent, so it has no switches",
                "leave Terminal as it is; the switches are for the agents",
            ));
        }
        let current = self.switches();
        if request.install_when_missing == Some(true) && !can_install(current.as_ref(), &kind) {
            return Err(invalid_field(
                "install_when_missing",
                Problem::NotAllowedHere,
                format!("Humata has no command to install {}", self.agents.person_name(&found.name)),
                "install it on each machine yourself, or give the agent an `install` command in agents.toml first",
            ));
        }
        let Some(path) = self.agents.settings_file() else {
            return Err(failure(
                Code::FailedPrecondition,
                pb::Error {
                    message: "this daemon was started without a home folder, so it has nowhere to keep the agent switches".to_owned(),
                    fix: "start humatad2 with a HOME (or --root) set, then change the switch again".to_owned(),
                    retry: None,
                    kind: None,
                },
            ));
        };
        let mut changes = Vec::new();
        let on_off = |b: bool| if b { "on" } else { "off" }.to_owned();
        if let Some(enabled) = request.enabled {
            let before = current.as_ref().is_none_or(|s| s.enabled(&kind));
            changes.push(("enabled".to_owned(), on_off(before), on_off(enabled)));
        }
        if let Some(install) = request.install_when_missing {
            let before = current.as_ref().is_some_and(|s| s.install_when_missing(&kind));
            changes.push(("install_when_missing".to_owned(), on_off(before), on_off(install)));
        }
        if let Some(asks) = request.asks_in_inbox {
            let before = current.as_ref().is_none_or(|s| s.asks_in_inbox(&kind));
            changes.push(("asks_in_inbox".to_owned(), on_off(before), on_off(asks)));
        }
        let path = path.to_owned();
        let change = Change {
            enabled: request.enabled,
            install_when_missing: request.install_when_missing,
            asks_in_inbox: request.asks_in_inbox,
        };
        let svc = self.clone();
        let name = kind.clone();
        let apply = move || async move {
            let write_name = name.clone();
            let written =
                tokio::task::spawn_blocking(move || agent_switches::set(&path, &write_name, change))
                    .await
                    .map_err(|error| internal(format!("saving the agent switches stopped: {error}"), ""))?
                    .map_err(|error| switches_failed(&error))?;
            svc.on_change.tell();
            let found = svc.agents.get(&name).map_err(|error| {
                internal(format!("the agent {name} went missing: {}", chain(&error)), "")
            })?;
            Ok(svc.definition(found, Some(&written)))
        };
        let Some(gate) = &self.gate else {
            return apply().await.map(Response::new);
        };
        let bytes = gate
            .gate_settings_change(
                &who,
                None,
                format!("Agent settings ({kind})"),
                changes,
                format!("Update agent switches for {kind}"),
                "humata.v1.AgentDefinition",
                Box::new(move || {
                    Box::pin(async move {
                        let def = apply().await?;
                        Ok(def.encode_to_vec())
                    })
                }),
            )
            .await?;
        let def = pb::AgentDefinition::decode(bytes.as_slice())
            .map_err(|error| internal(format!("decoding AgentDefinition failed: {error}"), ""))?;
        Ok(Response::new(def))
    }

    async fn add_agent(
        &self,
        request: Request<pb::AddAgentRequest>,
    ) -> Result<Response<pb::AgentDefinition>, Status> {
        if let (Some(gate), Some(who)) = (&self.gate, request.extensions().get::<Caller>().cloned()) {
            let req = request.into_inner();
            let key = req.idempotency_key.as_ref().map(|k| k.value.clone());
            let name = req
                .agent
                .as_ref()
                .and_then(|a| a.id.as_ref())
                .map(|id| id.value.clone())
                .filter(|id| !id.is_empty())
                .unwrap_or_else(|| "custom".to_owned());
            let bytes = gate
                .gate_settings_change(
                    &who,
                    key,
                    format!("Add agent ({name})"),
                    vec![("agent".to_owned(), "none".to_owned(), name)],
                    "Add custom agent definition".to_owned(),
                    "humata.v1.AgentDefinition",
                    Box::new(move || Box::pin(async move { Err(set_in_file("AddAgent")) })),
                )
                .await?;
            let def = pb::AgentDefinition::decode(bytes.as_slice())
                .map_err(|error| internal(format!("decoding AgentDefinition failed: {error}"), ""))?;
            return Ok(Response::new(def));
        }
        Err(set_in_file("AddAgent"))
    }

    async fn update_agent(
        &self,
        request: Request<pb::UpdateAgentRequest>,
    ) -> Result<Response<pb::AgentDefinition>, Status> {
        if let (Some(gate), Some(who)) = (&self.gate, request.extensions().get::<Caller>().cloned()) {
            let req = request.into_inner();
            let key = req.idempotency_key.as_ref().map(|k| k.value.clone());
            let name = req
                .agent
                .as_ref()
                .and_then(|a| a.id.as_ref())
                .map(|id| id.value.clone())
                .filter(|id| !id.is_empty())
                .unwrap_or_else(|| "custom".to_owned());
            let bytes = gate
                .gate_settings_change(
                    &who,
                    key,
                    format!("Update agent ({name})"),
                    vec![("agent".to_owned(), name.clone(), name)],
                    "Update agent definition".to_owned(),
                    "humata.v1.AgentDefinition",
                    Box::new(move || Box::pin(async move { Err(set_in_file("UpdateAgent")) })),
                )
                .await?;
            let def = pb::AgentDefinition::decode(bytes.as_slice())
                .map_err(|error| internal(format!("decoding AgentDefinition failed: {error}"), ""))?;
            return Ok(Response::new(def));
        }
        Err(set_in_file("UpdateAgent"))
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::Path;

    use humata2_daemon_ports::UserVia;
    use pb::agent_service_server::AgentService;

    use super::*;

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
        let agy = listed
            .iter()
            .find(|agent| id(agent).as_deref() == Some("agy"))
            .unwrap();
        assert_eq!(agy.name, "Antigravity");
        assert!(agy.supports_resume && agy.supports_turn_signal && !agy.supports_context_fork);
        assert_eq!(agy.turn_end_source, pb::TurnEndSource::OutputFallback as i32);
        assert_eq!(agy.launch_command, "agy");
        assert_eq!(agy.resume_arguments, "--continue");
        assert!(agy.can_install);
        assert_eq!(agy.install_command, "curl -fsSL https://antigravity.google/cli/install.sh | bash");
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
            asks_in_inbox: None,
        });
        request.extensions_mut().insert(caller);
        request
    }

    fn user() -> Caller {
        Caller::User(UserVia::Host)
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
    async fn asks_go_to_the_inbox_until_one_kind_turns_them_off() {
        let (svc, _dir) = with_switches("");
        let mut request = Request::new(pb::SetAgentEnabledRequest {
            agent: Some(pb::AgentId { value: "copilot".to_owned() }),
            asks_in_inbox: Some(false),
            ..Default::default()
        });
        request.extensions_mut().insert(user());

        let copilot = svc.set_agent_enabled(request).await.unwrap().into_inner();

        assert!(!copilot.asks_in_inbox);
        assert!(copilot.enabled);
        let listed = svc
            .list_agents(Request::new(pb::ListAgentsRequest::default()))
            .await
            .unwrap()
            .into_inner()
            .agents;
        let asks = |name: &str| {
            listed
                .iter()
                .find(|agent| ids(std::slice::from_ref(agent)) == [name])
                .map(|agent| agent.asks_in_inbox)
        };
        assert_eq!(asks("copilot"), Some(false));
        assert_eq!(asks("claude"), Some(true));
    }

    #[tokio::test]
    async fn a_written_switch_tells_the_change_hook() {
        let (svc, _dir) = with_switches("");
        let told = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let seen = told.clone();
        svc.on_change().set(move || {
            seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        });

        svc.set_agent_enabled(set("codex", Some(false), None, user())).await.unwrap();
        let _ = svc.set_agent_enabled(set("nope", Some(false), None, user())).await;

        assert_eq!(told.load(std::sync::atomic::Ordering::SeqCst), 1);
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

    #[tokio::test]
    async fn agent_set_agent_enabled_routes_through_settings_gate_with_diff() {
        struct RecordingGate {
            seen: std::sync::Mutex<Vec<(String, Vec<(String, String, String)>)>>,
        }

        #[tonic::async_trait]
        impl AgentSettingsGate for RecordingGate {
            async fn gate_settings_change(
                &self,
                _caller: &Caller,
                _key: Option<String>,
                scope: String,
                changes: Vec<(String, String, String)>,
                _reason: String,
                _type_name: &'static str,
                _run: Box<dyn FnOnce() -> BoxFuture<Result<Vec<u8>, Status>> + Send>,
            ) -> Result<Vec<u8>, Status> {
                self.seen.lock().unwrap().push((scope, changes));
                Err(Status::failed_precondition("pending in inbox"))
            }
        }

        let (svc, _dir) = with_switches("");
        let gate = Arc::new(RecordingGate {
            seen: std::sync::Mutex::new(Vec::new()),
        });
        let svc = svc.with_gate(gate.clone());

        let pending = svc
            .set_agent_enabled(set("codex", Some(false), None, Caller::Peer))
            .await
            .unwrap_err();
        assert_eq!(pending.code(), Code::FailedPrecondition);
        let recorded = gate.seen.lock().unwrap();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].0, "Agent settings (codex)");
        assert_eq!(
            recorded[0].1,
            vec![("enabled".to_owned(), "on".to_owned(), "off".to_owned())]
        );
    }
}
