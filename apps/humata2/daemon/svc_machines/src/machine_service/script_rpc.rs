//! MachineService's machine script calls (D687, D688): the rules and their
//! last runs, a check of the rules file, a run now as an Operation, and the
//! run log. Rules and scripts are the owner's files on the host: nothing
//! here writes them. A machine (peer) can't call any of it, an agent or
//! script sees nothing, and an agent's run now needs the channel's
//! `machine_script_run` switch.

use std::path::Path;
use std::sync::Arc;

use humata2_access::{AccessRequest, Call, Capability, Refusal, Target};
use humata2_daemon_ports::{Caller, Gated, Run};
use crate::pb;
use humata2_machines::{
    MachineScriptsError, Scope, ScriptRule, ScriptStep as Step, ScriptWarning, Trigger, scripts_dir,
};
use humata2_rpc_kit::{chain, error, failure, internal, invalid_field, missing, not_found};
use pb::invalid_field::Problem;
use tonic::{Code, Request, Response, Status};

use super::scripts::Installed;
use super::{MachineSvc, machine_id, machine_of, status_of};
use crate::machine_scripts::{
    Context, LogError, Outcome, RunNowError, RunRecord, SessionContext, StepRun,
};
use crate::ports::{
    Checked, MachineGuardPort, OperationProgress, OperationStarted, answer_gated, caller, deferred,
};

/// Longest label of a step on the rules list, as the log keeps it.
const LABEL_CHARS: usize = 80;

fn plain(message: String, fix: &str) -> pb::Error {
    pb::Error {
        message,
        fix: fix.to_owned(),
        retry: None,
        kind: None,
    }
}

fn not_running() -> Status {
    failure(
        Code::FailedPrecondition,
        plain(
            "machine scripts aren't running in this daemon".to_owned(),
            "restart Humata, then try again; if it keeps happening, update Humata",
        ),
    )
}

fn refused_peer() -> Status {
    let refusal = Refusal::PeerCannotCall;
    failure(
        Code::PermissionDenied,
        error(
            refusal.to_string(),
            "",
            pb::error::Kind::Denied(pb::Denied {
                source: pb::denied::Source::Unspecified as i32,
                capability: 0,
                request: None,
                user_note: String::new(),
            }),
        ),
    )
}

fn refused_user_only() -> Status {
    let refusal = Refusal::UserOnly { capability: None };
    failure(
        Code::PermissionDenied,
        error(
            refusal.to_string(),
            "",
            pb::error::Kind::UserOnly(pb::UserOnly { capability: 0 }),
        ),
    )
}

/// Whether the caller sees machine scripts: the user does, an agent or a
/// script sees none, and a peer can't ask.
fn sees(caller: &Caller) -> Result<bool, Status> {
    match caller {
        Caller::Peer => Err(refused_peer()),
        Caller::User(_) => Ok(true),
        Caller::Agent(_) | Caller::Script(_) => Ok(false),
    }
}

/// The rule check for run now: the user always may; an agent or script
/// needs the channel's `machine_script_run` switch (never by default).
fn run_gate(
    guard: &dyn MachineGuardPort,
    caller: &Caller,
    approved: bool,
) -> Result<Option<Box<AccessRequest>>, Status> {
    match caller {
        Caller::Peer => Err(refused_peer()),
        Caller::User(_) => Ok(None),
        Caller::Agent(_) | Caller::Script(_) => {
            if approved {
                return Ok(None);
            }
            match guard.check(
                caller,
                &Call::Use {
                    capability: Capability::MachineScriptRun,
                    target: Target::Global,
                },
            )? {
                Checked::Allowed => Ok(None),
                Checked::Ask(asked) => Ok(Some(Box::new(asked))),
            }
        }
    }
}

fn short(text: &str) -> String {
    text.chars().take(LABEL_CHARS).collect()
}

/// A step's kind and what the page shows for it.
fn describe(step: &Step) -> (&'static str, String) {
    match step {
        Step::Run { script, args } => {
            let mut words = vec![script.as_str().to_owned()];
            words.extend(args.iter().cloned());
            ("run", short(&words.join(" ")))
        }
        Step::Humata { args } => ("humata", short(&args.join(" "))),
        Step::Start { agent, channel, .. } => ("start", short(&format!("{agent} in {channel}"))),
        Step::Send { text, .. } => ("send", short(text)),
    }
}

fn rule_pb(rule: &ScriptRule) -> pb::MachineScriptRule {
    pb::MachineScriptRule {
        name: rule.name.clone(),
        triggers: rule
            .on
            .iter()
            .map(|trigger| trigger.as_str().to_owned())
            .collect(),
        machines: match &rule.scope {
            Scope::All => Vec::new(),
            Scope::Named(names) => names.clone(),
        },
        once: rule.once,
        steps: rule
            .steps
            .iter()
            .map(|step| {
                let (kind, label) = describe(step);
                pb::MachineScriptStep {
                    kind: kind.to_owned(),
                    label,
                }
            })
            .collect(),
    }
}

fn outcome_pb(outcome: Outcome) -> pb::MachineScriptOutcome {
    match outcome {
        Outcome::Passed => pb::MachineScriptOutcome::Passed,
        Outcome::Failed => pb::MachineScriptOutcome::Failed,
        Outcome::TimedOut => pb::MachineScriptOutcome::TimedOut,
        Outcome::Skipped => pb::MachineScriptOutcome::Skipped,
    }
}

fn step_run_pb(step: &StepRun) -> pb::MachineScriptStepRun {
    pb::MachineScriptStepRun {
        index: step.index,
        kind: step.kind.to_owned(),
        label: step.label.clone(),
        outcome: outcome_pb(step.outcome) as i32,
        exit: step.exit,
        tail: step.tail.clone(),
        error: step.error.clone().unwrap_or_default(),
        took_ms: step.took_ms,
    }
}

fn run_pb(run: &RunRecord) -> pb::MachineScriptRun {
    pb::MachineScriptRun {
        id: run.id.clone(),
        machine: Some(machine_id(&run.machine)),
        rule: run.rule.clone(),
        trigger: run.trigger.as_str().to_owned(),
        session: run
            .session
            .as_ref()
            .map(|session| pb::SessionId {
                value: session.clone(),
            }),
        started_at: Some(humata2_grpc_core::Timestamp {
            seconds: run.started_at_ms.div_euclid(1000),
            nanos: i32::try_from(run.started_at_ms.rem_euclid(1000) * 1_000_000).unwrap_or(0),
        }),
        took_ms: run.took_ms,
        outcome: outcome_pb(run.outcome) as i32,
        steps: run.steps.iter().map(step_run_pb).collect(),
    }
}

/// The file names in the scripts folder, sorted; none when it isn't there.
fn script_names(home: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(scripts_dir(home)) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_file())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect();
    names.sort();
    names
}

/// The rules, the last run of each on each machine it applies to
/// (`only`, when given), and the script names.
fn list(
    installed: &Installed,
    machines: &[String],
    only: Option<&str>,
) -> pb::ListMachineScriptsResponse {
    let mut response = pb::ListMachineScriptsResponse {
        scripts: script_names(&installed.home),
        ..pb::ListMachineScriptsResponse::default()
    };
    match installed.scripts.rules() {
        Err(err) => {
            response.problem = Some(plain(
                err.to_string(),
                "fix the file ~/.humata2/config/machine-scripts.toml, then run `humata machine script check`",
            ));
        }
        Ok(rules) => {
            for rule in &rules.rules {
                for machine in machines.iter().filter(|machine| {
                    rule.scope.includes(machine)
                        && only.is_none_or(|only| only == machine.as_str())
                }) {
                    if let Ok(last) = installed.scripts.runs(machine, Some(&rule.name), 1) {
                        response.last_runs.extend(last.iter().map(run_pb));
                    }
                }
            }
            response.rules = rules.rules.iter().map(rule_pb).collect();
        }
    }
    response
}

fn warning_text(warning: &ScriptWarning) -> String {
    match warning {
        ScriptWarning::UnknownMachine { rule, machine } => format!(
            "rule {rule:?} names the machine {machine:?}, which isn't a machine yet; the rule applies once a machine of that name is added"
        ),
    }
}

/// Every problem of the rules file, and what is worth a warning.
fn check(installed: &Installed) -> pb::CheckMachineScriptsResponse {
    match installed.scripts.rules() {
        Ok(rules) => pb::CheckMachineScriptsResponse {
            valid: true,
            problems: Vec::new(),
            warnings: rules.warnings.iter().map(warning_text).collect(),
        },
        Err(MachineScriptsError::Invalid { problems }) => pb::CheckMachineScriptsResponse {
            valid: false,
            problems: problems.iter().map(ToString::to_string).collect(),
            warnings: Vec::new(),
        },
        Err(err) => pb::CheckMachineScriptsResponse {
            valid: false,
            problems: vec![err.to_string()],
            warnings: Vec::new(),
        },
    }
}

fn rules_status(err: &MachineScriptsError) -> Status {
    failure(
        Code::FailedPrecondition,
        plain(
            err.to_string(),
            "fix the file ~/.humata2/config/machine-scripts.toml, then run `humata machine script check`",
        ),
    )
}

/// The rule named, when it is for `machine`.
fn find_rule(installed: &Installed, rule: &str, machine: &str) -> Result<ScriptRule, Status> {
    let rules = installed
        .scripts
        .rules()
        .map_err(|err| rules_status(&err))?;
    let Some(found) = rules
        .rules
        .into_iter()
        .find(|candidate| candidate.name == rule)
    else {
        return Err(not_found(
            pb::EntityKind::Unspecified,
            rule,
            format!("there is no rule named {rule:?} in the machine scripts file"),
            "list the rules with `humata machine script list`, or add the rule to ~/.humata2/config/machine-scripts.toml",
        ));
    };
    if !found.scope.includes(machine) {
        return Err(invalid_field(
            "machine",
            Problem::NotAllowedHere,
            format!("the rule {rule:?} isn't for {machine}: its `machines` list leaves this machine out"),
            "add the machine to the rule's list, or run a rule that applies to it",
        ));
    }
    Ok(found)
}

fn log_status(err: &LogError) -> Status {
    match err {
        LogError::BadMachine { .. } => invalid_field(
            "machine",
            Problem::UnknownValue,
            err.to_string(),
            "use the machine's name as the machine list shows it",
        ),
        _ => internal(chain(err), ""),
    }
}

/// The machine's runs, newest first; a limit of 0 is the whole log.
fn runs(
    installed: &Installed,
    machine: &str,
    rule: Option<&str>,
    limit: u32,
) -> Result<Vec<pb::MachineScriptRun>, Status> {
    let limit = if limit == 0 {
        usize::MAX
    } else {
        usize::try_from(limit).unwrap_or(usize::MAX)
    };
    let runs = installed
        .scripts
        .runs(machine, rule, limit)
        .map_err(|err| log_status(&err))?;
    Ok(runs.iter().map(run_pb).collect())
}

/// Ends the run's operation with how the run came out.
fn end_operation(progress: Arc<dyn OperationProgress>, ran: Result<RunRecord, RunNowError>) {
    match ran {
        Ok(run) if run.outcome == Outcome::Passed => {
            progress.step(0, pb::StepState::Done, String::new());
            progress.finish_ok("", Vec::new());
        }
        Ok(run) => {
            let failed = run
                .steps
                .iter()
                .find(|step| !matches!(step.outcome, Outcome::Passed | Outcome::Skipped));
            let (what, tail) = match failed {
                Some(step) => (
                    format!(
                        "step {} ({} {}): {}",
                        step.index + 1,
                        step.kind,
                        step.label,
                        step.error.clone().unwrap_or_default()
                    ),
                    step.tail.join("\n"),
                ),
                None => (String::new(), String::new()),
            };
            progress.step(0, pb::StepState::Failed, tail);
            progress.finish_err(plain(
                format!(
                    "the rule {} didn't pass on {}: {what}",
                    run.rule, run.machine
                ),
                "read the run with `humata machine script log`, fix the script or the rule, then run it again",
            ));
        }
        Err(err) => {
            progress.step(0, pb::StepState::Failed, String::new());
            progress.finish_err(plain(err.to_string(), ""));
        }
    }
}

impl MachineSvc {
    fn installed(&self) -> Result<&Installed, Status> {
        self.work.scripts.get().ok_or_else(not_running)
    }

    fn known_machines(&self) -> Result<Vec<String>, Status> {
        self.work
            .machines
            .list()
            .map(|entries| entries.into_iter().map(|entry| entry.name).collect())
            .map_err(|err| status_of(&err))
    }

    pub(super) async fn rpc_list_machine_scripts(
        &self,
        request: Request<pb::ListMachineScriptsRequest>,
    ) -> Result<Response<pb::ListMachineScriptsResponse>, Status> {
        let caller = caller(&request)?;
        let only = request
            .into_inner()
            .machine
            .map(|id| id.value)
            .filter(|name| !name.is_empty());
        if !sees(&caller)? {
            return Ok(Response::new(pb::ListMachineScriptsResponse::default()));
        }
        let machines = self.known_machines()?;
        Ok(Response::new(list(
            self.installed()?,
            &machines,
            only.as_deref(),
        )))
    }

    pub(super) async fn rpc_check_machine_scripts(
        &self,
        request: Request<pb::CheckMachineScriptsRequest>,
    ) -> Result<Response<pb::CheckMachineScriptsResponse>, Status> {
        let caller = caller(&request)?;
        if !sees(&caller)? {
            return Err(refused_user_only());
        }
        Ok(Response::new(check(self.installed()?)))
    }

    pub(super) async fn rpc_list_machine_script_runs(
        &self,
        request: Request<pb::ListMachineScriptRunsRequest>,
    ) -> Result<Response<pb::ListMachineScriptRunsResponse>, Status> {
        let caller = caller(&request)?;
        let request = request.into_inner();
        let machine = machine_of(request.machine)?;
        if !sees(&caller)? {
            return Ok(Response::new(pb::ListMachineScriptRunsResponse::default()));
        }
        let rule = Some(request.rule.trim()).filter(|rule| !rule.is_empty());
        Ok(Response::new(pb::ListMachineScriptRunsResponse {
            runs: runs(self.installed()?, &machine, rule, request.limit)?,
        }))
    }

    pub(super) async fn rpc_run_machine_script(
        &self,
        request: Request<pb::RunMachineScriptRequest>,
    ) -> Result<Response<pb::Operation>, Status> {
        let caller = caller(&request)?;
        let request = request.into_inner();
        let first = self.begin_run(&caller, request.clone(), false).await?;
        let (me, who) = (self.clone(), caller.clone());
        let run: Run = Box::new(move |_note| {
            Box::pin(async move {
                match me.begin_run(&who, request, true).await? {
                    Gated::Done(op) => Ok(deferred("humata.v1.Operation", &op)),
                    Gated::Ask(_) => Err(internal(
                        "approved machine script run asked for approval again",
                        "retry the call",
                    )),
                }
            })
        });
        answer_gated(
            self.inbox.as_ref(),
            &caller,
            "humata.v1.Operation",
            first,
            Some(run),
        )
        .await
    }

    /// Checks the call, then runs the rule in the background under an
    /// Operation that ends with the run.
    async fn begin_run(
        &self,
        caller: &Caller,
        request: pb::RunMachineScriptRequest,
        approved: bool,
    ) -> Result<Gated<pb::Operation>, Status> {
        if matches!(caller, Caller::Peer) {
            return Err(refused_peer());
        }
        let machine = machine_of(request.machine.clone())?;
        let rule = request.rule.trim().to_owned();
        if rule.is_empty() {
            return Err(missing("rule"));
        }
        if let Some(asked) = run_gate(self.guard.as_ref(), caller, approved)? {
            return Ok(Gated::Ask(asked));
        }
        if let Some(operation) = self
            .operations
            .existing(caller, request.idempotency_key.as_ref())?
        {
            return Ok(Gated::Done(operation));
        }
        self.work
            .machines
            .get(&machine)
            .map_err(|err| status_of(&err))?;
        let installed = self.installed()?;
        let found = find_rule(installed, &rule, &machine)?;
        // A session names the trigger it stands in for: the rule's own
        // session trigger, else a manual run that still sees the session.
        let session = request
            .session
            .as_ref()
            .map(|id| id.value.clone())
            .filter(|id| !id.is_empty());
        let trigger = match &session {
            Some(_) => found
                .on
                .iter()
                .copied()
                .find(|trigger| trigger.is_session())
                .unwrap_or(Trigger::Manual),
            None => Trigger::Manual,
        };
        let context = Context {
            machine: machine.clone(),
            trigger,
            session: session.map(|session| SessionContext {
                session,
                channel: String::new(),
                folder: String::new(),
                from: String::new(),
                to: machine.clone(),
            }),
        };
        let progress = match self.operations.start(
            caller,
            pb::OperationKind::MachineScript,
            request.idempotency_key,
            vec![format!("Running {rule} on {machine}")],
        )? {
            OperationStarted::Started(progress) => progress,
            OperationStarted::Existing(operation) => return Ok(Gated::Done(operation)),
        };
        let operation = progress.snapshot();
        let scripts = Arc::clone(&installed.scripts);
        progress.step(0, pb::StepState::Running, String::new());
        tokio::spawn(async move {
            let ran = scripts.run_now(&rule, context).await;
            end_operation(progress, ran);
        });
        Ok(Gated::Done(operation))
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::Mutex;
    use std::time::{Duration, SystemTime};

    use humata2_daemon_ports::{AgentCaller, UserVia};
    use humata2_machines::ScriptBody;
    use humata2_store::SessionRecord;

    use super::*;
    use crate::machine_scripts::{Fut, MachineScripts, Report, ShellOutput, StepError, Steps};

    const HERE: &str = "ma_host";

    struct DenyAllGuard;

    impl MachineGuardPort for DenyAllGuard {
        fn machine(&self) -> &str {
            HERE
        }
        fn check(&self, _caller: &Caller, _call: &Call) -> Result<Checked, Status> {
            Err(Status::permission_denied("denied"))
        }
        fn require(&self, _caller: &Caller, _call: &Call) -> Result<(), Status> {
            Err(Status::permission_denied("denied"))
        }
        fn principal(&self, _caller: &Caller) -> String {
            String::new()
        }
        fn is_present_user(&self, _caller: &Caller) -> bool {
            false
        }
    }

    struct Shell;

    impl Steps for Shell {
        fn shell<'a>(
            &'a self,
            _: &'a str,
            _: ScriptBody,
            _: Vec<String>,
            _: Vec<(String, String)>,
            _: Duration,
        ) -> Fut<'a, Result<ShellOutput, StepError>> {
            Box::pin(async {
                Ok(ShellOutput {
                    exit: Some(0),
                    timed_out: false,
                    tail: Vec::new(),
                })
            })
        }
        fn start<'a>(
            &'a self,
            _: &'a str,
            _: &'a str,
            _: &'a str,
            _: &'a str,
            _: &'a str,
        ) -> Fut<'a, Result<String, StepError>> {
            Box::pin(async { Err(StepError::NoSession) })
        }
        fn wait_idle<'a>(&'a self, _: &'a str, _: Duration) -> Fut<'a, Result<(), StepError>> {
            Box::pin(async { Ok(()) })
        }
        fn send<'a>(
            &'a self,
            _: &'a str,
            _: &'a str,
            _: bool,
            _: &'a str,
        ) -> Fut<'a, Result<(), StepError>> {
            Box::pin(async { Ok(()) })
        }
    }

    struct Told(Mutex<Vec<String>>);

    impl Report for Told {
        fn failed<'a>(&'a self, _: &'a RunRecord) -> Fut<'a, ()> {
            Box::pin(async {})
        }
        fn passed<'a>(&'a self, _: &'a RunRecord) -> Fut<'a, ()> {
            Box::pin(async {})
        }
        fn changed(&self, machine: &str) {
            self.0.lock().unwrap().push(machine.to_owned());
        }
    }

    fn installed(rules: &str) -> (tempfile::TempDir, Installed) {
        let home = tempfile::tempdir().unwrap();
        fs::create_dir_all(home.path().join("config")).unwrap();
        fs::create_dir_all(home.path().join("scripts")).unwrap();
        fs::write(home.path().join("scripts/setup.sh"), "true").unwrap();
        fs::write(home.path().join("config/machine-scripts.toml"), rules).unwrap();
        let scripts = MachineScripts::new(
            home.path().to_path_buf(),
            home.path().join("state"),
            Arc::new(Shell),
            Arc::new(Told(Mutex::new(Vec::new()))),
            Arc::new(|| vec!["box".to_owned(), "other".to_owned()]),
            Arc::new(SystemTime::now),
        );
        let home_path = home.path().to_path_buf();
        (home, Installed { scripts, home: home_path })
    }

    const ONE_RULE: &str = "[[rule]]\nname = \"setup\"\non = [\"connected\"]\nmachines = [\"box\"]\n[[rule.step]]\nrun = \"setup.sh\"\n";

    #[test]
    fn the_rules_are_listed_with_their_steps_and_the_script_names() {
        let (_home, installed) = installed(ONE_RULE);
        let listed = list(&installed, &["box".to_owned(), "other".to_owned()], None);
        assert!(listed.problem.is_none());
        assert_eq!(listed.rules.len(), 1);
        assert_eq!(listed.rules[0].triggers, ["connected"]);
        assert_eq!(listed.rules[0].machines, ["box"]);
        assert_eq!(listed.rules[0].steps[0].kind, "run");
        assert_eq!(listed.scripts, ["setup.sh"]);
    }

    #[test]
    fn a_file_that_cant_be_used_is_a_problem_not_a_failure_of_the_list() {
        let (_home, installed) = installed("[[rule]]\nname = \"Bad Name\"\n");
        let listed = list(&installed, &["box".to_owned()], None);
        assert!(listed.problem.is_some());
        assert!(listed.rules.is_empty());
    }

    #[test]
    fn check_says_valid_for_a_good_file_and_lists_problems_for_a_bad_one() {
        let (_good_home, good) = installed(ONE_RULE);
        assert!(check(&good).valid);
        let (_bad_home, bad) = installed("[[rule]]\nname = \"x\"\non = [\"connected\"]\n");
        let checked = check(&bad);
        assert!(!checked.valid);
        assert!(!checked.problems.is_empty());
    }

    #[test]
    fn check_warns_of_a_machine_that_isnt_one_yet_without_failing() {
        let rules = ONE_RULE.replace("\"box\"", "\"later\"");
        let (_home, installed) = installed(&rules);
        let checked = check(&installed);
        assert!(checked.valid);
        assert_eq!(checked.warnings.len(), 1);
    }

    #[test]
    fn running_an_unknown_rule_is_not_found_and_a_rule_for_other_machines_is_refused() {
        let (_home, installed) = installed(ONE_RULE);
        assert_eq!(
            find_rule(&installed, "nope", "box").unwrap_err().code(),
            Code::NotFound
        );
        assert_eq!(
            find_rule(&installed, "setup", "other").unwrap_err().code(),
            Code::InvalidArgument
        );
        assert!(find_rule(&installed, "setup", "box").is_ok());
    }

    #[tokio::test]
    async fn a_run_is_in_the_log_with_its_steps() {
        let (_home, installed) = installed(ONE_RULE);
        let context = Context {
            machine: "box".to_owned(),
            trigger: Trigger::Manual,
            session: None,
        };
        installed.scripts.run_now("setup", context).await.unwrap();

        let logged = runs(&installed, "box", None, 0).unwrap();
        assert_eq!(logged.len(), 1);
        assert_eq!(logged[0].outcome(), pb::MachineScriptOutcome::Passed);
        assert_eq!(logged[0].trigger, "manual");
        assert_eq!(logged[0].steps.len(), 1);
        assert!(runs(&installed, "other", None, 0).unwrap().is_empty());
        assert!(
            runs(&installed, "box", Some("nothing"), 0)
                .unwrap()
                .is_empty()
        );
    }

    fn agent() -> Caller {
        Caller::Agent(AgentCaller {
            session: humata2_access::SessionRef {
                id: humata2_access::SessionId::new("se_a"),
                channel: humata2_access::ChannelId::new("ch_a"),
                machine: humata2_access::MachineId::new(HERE),
            },
            agent: "claude".to_owned(),
            title: "asker".to_owned(),
            method: String::new(),
            context_only_parent: None,
            desk: None,
            review_push: None,
            channel_desk: None,
        })
    }

    #[test]
    fn an_agent_without_the_switch_is_refused_and_the_user_never_is() {
        let guard = DenyAllGuard;
        let refused = run_gate(&guard, &agent(), false).err().unwrap();
        assert_eq!(refused.code(), Code::PermissionDenied);
        assert!(
            run_gate(&guard, &Caller::User(UserVia::App), false)
                .unwrap()
                .is_none()
        );
        assert!(run_gate(&guard, &Caller::Peer, false).is_err());
    }

    #[test]
    fn only_the_user_sees_machine_scripts() {
        assert!(sees(&Caller::User(UserVia::App)).unwrap());
        assert!(!sees(&agent()).unwrap());
        assert!(sees(&Caller::Peer).is_err());
    }
}
