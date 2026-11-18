//! The live side of machine scripts (D680-D688): what each step does on a
//! real machine, and where a run's result goes. Steps drive the machines
//! (`Machines::machine_script`), the session service (a `start` step) and
//! the terminal (a `send` step); the report adapter keeps one Inbox host
//! ask per (machine, rule) and tells the feed that a machine's run log
//! changed. The run engine itself (`engine.rs`) knows none of this.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError, Weak};
use std::time::{Duration, SystemTime};

use humata2_grpc::humata::v1 as pb;
use humata2_machines::{Exit, ManageError, Runner, ScriptBody, SystemRunner, Trigger};
use tokio::sync::broadcast;
use tonic::{Code, Status};

use super::{Context, Fut, MachineScripts, Outcome, Report, RunRecord, SessionContext, ShellOutput, StepError, Steps};
use crate::events::Feed;
use crate::inbox::{Deferred, Inbox, Run};
use crate::machine_service::{Work, machine_id};
use crate::session_service::SessionSvc;
use crate::status;
use crate::terminal::Core;

/// How many lines of a script's output a step record keeps (D686).
const TAIL_LINES: usize = 40;

/// What the live steps and report are built from.
pub(crate) struct Parts {
    pub(crate) work: Arc<Work>,
    pub(crate) sessions: SessionSvc,
    pub(crate) terminal: Arc<Core>,
    pub(crate) feed: Arc<Feed>,
    pub(crate) inbox: Arc<Inbox>,
    /// `~/.humata2`, where the rules file and the scripts are.
    pub(crate) humata_home: PathBuf,
    /// The daemon's state folder, where the run log is kept.
    pub(crate) state: PathBuf,
}

/// The run engine over the live steps and the Inbox and feed report.
pub(crate) fn build(parts: Parts) -> Arc<MachineScripts> {
    let engine = Arc::new(OnceLock::new());
    let steps = Arc::new(LiveSteps {
        work: Arc::clone(&parts.work),
        sessions: parts.sessions,
        terminal: parts.terminal,
        feed: Arc::clone(&parts.feed),
    });
    let report = Arc::new(LiveReport::new(parts.inbox, parts.feed, Arc::clone(&engine)));
    let work = parts.work;
    let machines = Arc::new(move || match work.machines.list() {
        Ok(entries) => entries.into_iter().map(|entry| entry.name).collect(),
        Err(_) => Vec::new(),
    });
    let scripts = MachineScripts::new(
        parts.humata_home,
        parts.state,
        steps,
        report,
        machines,
        Arc::new(SystemTime::now),
    );
    let _ = engine.set(Arc::downgrade(&scripts));
    scripts
}

/// Runs the session-leaving rules for `context` and waits for them, but no
/// longer than `cap`: scripts never hold a move for long and never fail it
/// (D681). True when they ended within the cap. What is left running goes
/// on in the background and is logged like any run.
pub(crate) async fn leaving(scripts: &Arc<MachineScripts>, context: Context, cap: Duration) -> bool {
    tokio::time::timeout(cap, scripts.fire_and_wait(context)).await.is_ok()
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn plain(message: String, fix: &str) -> pb::Error {
    pb::Error {
        message,
        fix: fix.to_owned(),
        retry: None,
        kind: None,
    }
}

// ---------------------------------------------------------------------------
// The steps.

struct LiveSteps {
    work: Arc<Work>,
    sessions: SessionSvc,
    terminal: Arc<Core>,
    feed: Arc<Feed>,
}

/// How a program's end reads as a step's: the exit code, none for a signal,
/// and whether the time limit ended it.
fn shell_output(output: &humata2_machines::Output) -> ShellOutput {
    let (exit, timed_out) = match output.exit {
        Exit::Code(code) => (Some(code), false),
        Exit::Signal(_) => (None, false),
        Exit::TimedOut => (None, true),
    };
    ShellOutput {
        exit,
        timed_out,
        tail: output.tail(TAIL_LINES).lines().map(str::to_owned).collect(),
    }
}

/// Whether `event` is a change of session `session`.
fn is_session_event(event: &pb::Event, session: &str) -> bool {
    matches!(
        &event.payload,
        Some(pb::event::Payload::Session(shown)) if shown.id.as_ref().is_some_and(|id| id.value == session)
    )
}

impl Steps for LiveSteps {
    fn shell<'a>(
        &'a self,
        machine: &'a str,
        body: ScriptBody,
        args: Vec<String>,
        env: Vec<(String, String)>,
        timeout: Duration,
    ) -> Fut<'a, Result<ShellOutput, StepError>> {
        Box::pin(async move {
            let work = Arc::clone(&self.work);
            let named = machine.to_owned();
            let ran = tokio::task::spawn_blocking(move || {
                let shell = |source: ManageError| StepError::Shell {
                    machine: named.clone(),
                    source,
                };
                let invocation = work
                    .machines
                    .machine_script(&named, &body, &args, &env, timeout)
                    .map_err(shell)?;
                SystemRunner::new(&())
                    .run(&invocation)
                    .map_err(|error| StepError::Shell {
                        machine: named.clone(),
                        source: ManageError::Run {
                            machine: named.clone(),
                            source: error,
                        },
                    })
            })
            .await
            .map_err(|error| StepError::Interrupted {
                what: format!("the script on {machine}"),
                message: error.to_string(),
            })??;
            Ok(shell_output(&ran))
        })
    }

    fn start<'a>(
        &'a self,
        machine: &'a str,
        agent: &'a str,
        channel: &'a str,
        title: &'a str,
        key: &'a str,
    ) -> Fut<'a, Result<String, StepError>> {
        Box::pin(async move {
            self.sessions
                .start_for_script(machine, agent, channel, title, key)
                .await
                .map_err(|failure| {
                    let fix = status::error_of(&failure).map(|error| error.fix).unwrap_or_default();
                    let message = if fix.is_empty() {
                        failure.message().to_owned()
                    } else {
                        format!("{} ({fix})", failure.message())
                    };
                    StepError::Start {
                        machine: machine.to_owned(),
                        agent: agent.to_owned(),
                        message,
                    }
                })
        })
    }

    fn wait_idle<'a>(&'a self, session: &'a str, timeout: Duration) -> Fut<'a, Result<(), StepError>> {
        Box::pin(async move {
            let fail = |message: String| StepError::WaitIdle {
                session: session.to_owned(),
                message,
            };
            // Subscribed before the first look, so no change falls between.
            let (_, mut live) = self.feed.subscribe(None);
            let waiting = async {
                loop {
                    let view = self
                        .sessions
                        .script_view(session)
                        .await
                        .map_err(|failure| fail(failure.message().to_owned()))?;
                    if view.turn_end == pb::TurnEndSource::None {
                        return Err(fail(format!("can't tell when {} is ready", view.agent)));
                    }
                    match view.status {
                        pb::SessionStatus::Idle | pb::SessionStatus::ProbablyIdle => return Ok(()),
                        pb::SessionStatus::Exited
                        | pb::SessionStatus::ResumeFailed
                        | pb::SessionStatus::Archived
                        | pb::SessionStatus::MachineStopped => {
                            return Err(fail("the session isn't running".to_owned()));
                        }
                        _ => {}
                    }
                    // Nothing polls: the next change of this session wakes it.
                    loop {
                        match live.recv().await {
                            Ok(event) if is_session_event(&event, session) => break,
                            Ok(_) => {}
                            Err(broadcast::error::RecvError::Lagged(_)) => break,
                            Err(broadcast::error::RecvError::Closed) => {
                                return Err(fail("the daemon is stopping".to_owned()));
                            }
                        }
                    }
                }
            };
            tokio::time::timeout(timeout, waiting)
                .await
                .unwrap_or_else(|_| Err(fail(format!("it was still busy after {} seconds", timeout.as_secs()))))
        })
    }

    fn send<'a>(&'a self, session: &'a str, text: &'a str, enter: bool, key: &'a str) -> Fut<'a, Result<(), StepError>> {
        Box::pin(async move {
            self.terminal
                .type_for_script(session, text, enter, key)
                .await
                .map_err(|failure| StepError::Send {
                    session: session.to_owned(),
                    message: failure.message().to_owned(),
                })
        })
    }
}

// ---------------------------------------------------------------------------
// The report.

/// Posts a failed run to the Inbox, withdraws it on a pass, and tells the
/// feed (D686).
pub(crate) struct LiveReport {
    inbox: Arc<Inbox>,
    feed: Arc<Feed>,
    /// The engine, for Retry; set once it is built.
    engine: Arc<OnceLock<Weak<MachineScripts>>>,
    /// The id of each machine's last run, for the feed.
    last: Mutex<HashMap<String, String>>,
}

/// The key one request for `rule` on `machine` is held under.
fn ask_key(machine: &str, rule: &str) -> String {
    format!("machine-script/{machine}/{rule}")
}

/// The first step that didn't pass.
fn failed_step(run: &RunRecord) -> Option<&super::StepRun> {
    run.steps
        .iter()
        .find(|step| !matches!(step.outcome, Outcome::Passed | Outcome::Skipped))
}

/// The Inbox card of a failed run: the failed step, its exit, the end of
/// its output and the trigger (D686).
fn failure_card(run: &RunRecord) -> pb::InboxRequest {
    let (step, command, tail) = match failed_step(run) {
        Some(step) => {
            let exit = step.exit.map(|code| format!(" (exit {code})")).unwrap_or_default();
            let why = step.error.clone().unwrap_or_default();
            (
                format!("step {} ({} {}){exit}: {why}", step.index + 1, step.kind, step.label),
                format!("{} {}", step.kind, step.label),
                step.tail.join("\n"),
            )
        }
        None => (String::new(), String::new(), String::new()),
    };
    let reason = format!(
        "The rule {} ran on {} because of {} and stopped at {step}. Approve this to run the rule again, or deny it to leave things as they are. The next pass of the rule clears this request.",
        run.rule,
        run.machine,
        run.trigger.as_str()
    );
    pb::InboxRequest {
        action_title: format!("Script {} failed on {}", run.rule, run.machine),
        reason,
        preview: Some(pb::RequestPreview {
            preview: Some(pb::request_preview::Preview::Command(pb::CommandPreview {
                machine: Some(machine_id(&run.machine)),
                command,
                notes: tail,
            })),
        }),
        operation: "machine.script".to_owned(),
        ..Default::default()
    }
}

/// What approving the request does: the rule runs again on the machine.
async fn run_again(
    engine: Arc<OnceLock<Weak<MachineScripts>>>,
    machine: String,
    rule: String,
    trigger: Trigger,
    session: Option<String>,
) -> Result<Deferred, Status> {
    let Some(scripts) = engine.get().and_then(Weak::upgrade) else {
        return Err(status::internal(
            "the machine scripts engine has stopped, so the rule wasn't run again; restart Humata and run it from the machine's page",
            "",
        ));
    };
    let context = Context {
        machine,
        trigger,
        // Only the session's id is kept with a run: its variables that
        // need more are left out of this run.
        session: session.map(|session| SessionContext {
            session,
            channel: String::new(),
            folder: String::new(),
            from: String::new(),
            to: String::new(),
        }),
    };
    scripts
        .run_now(&rule, context)
        .await
        .map_err(|error| status::failure(Code::FailedPrecondition, plain(error.to_string(), "")))?;
    // A failure of the run is reported again, as a new request.
    Ok(Deferred {
        result: Vec::new(),
        result_type: String::new(),
    })
}

impl LiveReport {
    pub(crate) fn new(inbox: Arc<Inbox>, feed: Arc<Feed>, engine: Arc<OnceLock<Weak<MachineScripts>>>) -> Self {
        Self {
            inbox,
            feed,
            engine,
            last: Mutex::new(HashMap::new()),
        }
    }

    fn remember(&self, run: &RunRecord) {
        lock(&self.last).insert(run.machine.clone(), run.id.clone());
    }
}

impl Report for LiveReport {
    fn failed<'a>(&'a self, run: &'a RunRecord) -> Fut<'a, ()> {
        Box::pin(async move {
            self.remember(run);
            let (engine, machine, rule) = (Arc::clone(&self.engine), run.machine.clone(), run.rule.clone());
            let (trigger, session) = (run.trigger, run.session.clone());
            let again: Run = Box::new(move |_| Box::pin(run_again(engine, machine, rule, trigger, session)));
            if let Err(error) = self
                .inbox
                .post_host_ask(&ask_key(&run.machine, &run.rule), failure_card(run), again)
                .await
            {
                crate::log::say!(
                    "humatad2: couldn't put the failed machine script {} on {} in the Inbox: {}; its run is in the machine's script log",
                    run.rule,
                    run.machine,
                    error.message()
                );
            }
        })
    }

    fn passed<'a>(&'a self, run: &'a RunRecord) -> Fut<'a, ()> {
        Box::pin(async move {
            self.remember(run);
            self.inbox
                .withdraw_host_ask(&ask_key(&run.machine, &run.rule), pb::WithdrawReason::Resolved)
                .await;
        })
    }

    fn changed(&self, machine: &str) {
        let run_id = lock(&self.last).get(machine).cloned().unwrap_or_default();
        self.feed.publish(
            pb::Change::Updated,
            pb::event::Payload::MachineScriptRun(pb::MachineScriptRunChanged {
                machine: Some(machine_id(machine)),
                run_id,
            }),
            None,
        );
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use humata2_grpc::humata::v1::inbox_service_server::InboxService;
    use humata2_store::Store;

    use super::*;
    use crate::guard::Guard;
    use crate::identity::{Caller, UserVia};
    use crate::machine_scripts::StepRun;

    const HOST: &str = "ma_host";

    fn inbox(directory: &std::path::Path, feed: &Arc<Feed>) -> Arc<Inbox> {
        let store = Arc::new(Store::open(directory).unwrap());
        let guard = Arc::new(Guard::new(HOST, pb::DaemonRole::Host));
        Arc::new(Inbox::new(guard, store, HOST.to_owned(), Arc::clone(feed)))
    }

    fn run(rule: &str, outcome: Outcome) -> RunRecord {
        let failed = outcome != Outcome::Passed;
        RunRecord {
            id: format!("1-{rule}"),
            machine: "box".to_owned(),
            rule: rule.to_owned(),
            trigger: Trigger::Connected,
            session: None,
            started_at_ms: 1,
            took_ms: 1,
            outcome,
            steps: vec![StepRun {
                index: 0,
                kind: "run",
                label: "setup.sh".to_owned(),
                outcome,
                exit: failed.then_some(3),
                tail: vec!["boom".to_owned()],
                error: failed.then(|| "exited with 3".to_owned()),
                took_ms: 1,
            }],
        }
    }

    async fn pending(inbox: &Inbox) -> Vec<pb::InboxRequest> {
        let mut request = tonic::Request::new(pb::ListRequestsRequest::default());
        request.extensions_mut().insert(Caller::User(UserVia::App));
        inbox
            .list_requests(request)
            .await
            .unwrap()
            .into_inner()
            .requests
            .into_iter()
            .filter(|request| request.state() == pb::RequestState::Pending)
            .collect()
    }

    fn report(inbox: Arc<Inbox>, feed: Arc<Feed>) -> LiveReport {
        LiveReport::new(inbox, feed, Arc::new(OnceLock::new()))
    }

    #[tokio::test]
    async fn a_failed_run_posts_one_request_a_second_failure_replaces_and_a_pass_withdraws() {
        let directory = tempfile::tempdir().unwrap();
        let feed = Arc::new(Feed::default());
        let inbox = inbox(directory.path(), &feed);
        let report = report(Arc::clone(&inbox), feed);

        report.failed(&run("setup", Outcome::Failed)).await;
        report.failed(&run("setup", Outcome::Failed)).await;
        report.failed(&run("other", Outcome::TimedOut)).await;
        let held = pending(&inbox).await;
        assert_eq!(held.len(), 2, "one request for each (machine, rule)");
        assert!(held.iter().all(|request| request.vip));

        report.passed(&run("setup", Outcome::Passed)).await;
        assert_eq!(pending(&inbox).await.len(), 1, "a pass withdraws only its own rule's request");
    }

    #[tokio::test]
    async fn a_change_goes_on_the_feed_with_the_machine_and_its_last_run() {
        let directory = tempfile::tempdir().unwrap();
        let feed = Arc::new(Feed::default());
        let report = report(inbox(directory.path(), &feed), Arc::clone(&feed));
        let (_, mut live) = feed.subscribe(None);

        report.passed(&run("setup", Outcome::Passed)).await;
        report.changed("box");

        let event = live.recv().await.unwrap();
        match &event.payload {
            Some(pb::event::Payload::MachineScriptRun(changed)) => {
                assert_eq!(changed.machine.as_ref().map(|id| id.value.as_str()), Some("box"));
                assert_eq!(changed.run_id, "1-setup");
            }
            other => panic!("expected a machine script run change, got {other:?}"),
        }
    }

    /// Steps that finish at once, or never when the script text says "hang".
    struct FakeSteps {
        ran: Mutex<Vec<String>>,
    }

    impl Steps for FakeSteps {
        fn shell<'a>(
            &'a self,
            _machine: &'a str,
            body: ScriptBody,
            _args: Vec<String>,
            _env: Vec<(String, String)>,
            _timeout: Duration,
        ) -> Fut<'a, Result<ShellOutput, StepError>> {
            Box::pin(async move {
                let text = match &body {
                    ScriptBody::Bash(bytes) => String::from_utf8_lossy(bytes).trim().to_owned(),
                    ScriptBody::Humata(args) => args.join(" "),
                };
                lock(&self.ran).push(text.clone());
                if text == "hang" {
                    tokio::time::sleep(Duration::from_secs(3600)).await;
                }
                Ok(ShellOutput { exit: Some(0), timed_out: false, tail: Vec::new() })
            })
        }
        fn start<'a>(&'a self, _: &'a str, _: &'a str, _: &'a str, _: &'a str, _: &'a str) -> Fut<'a, Result<String, StepError>> {
            Box::pin(async { Err(StepError::NoSession) })
        }
        fn wait_idle<'a>(&'a self, _: &'a str, _: Duration) -> Fut<'a, Result<(), StepError>> {
            Box::pin(async { Ok(()) })
        }
        fn send<'a>(&'a self, _: &'a str, _: &'a str, _: bool, _: &'a str) -> Fut<'a, Result<(), StepError>> {
            Box::pin(async { Ok(()) })
        }
    }

    struct Quiet;

    impl Report for Quiet {
        fn failed<'a>(&'a self, _: &'a RunRecord) -> Fut<'a, ()> {
            Box::pin(async {})
        }
        fn passed<'a>(&'a self, _: &'a RunRecord) -> Fut<'a, ()> {
            Box::pin(async {})
        }
        fn changed(&self, _: &str) {}
    }

    fn engine(script: &str) -> (tempfile::TempDir, Arc<MachineScripts>, Arc<FakeSteps>) {
        let home = tempfile::tempdir().unwrap();
        fs::create_dir_all(home.path().join("config")).unwrap();
        fs::create_dir_all(home.path().join("scripts")).unwrap();
        fs::write(home.path().join("scripts/go.sh"), script).unwrap();
        fs::write(
            home.path().join("config/machine-scripts.toml"),
            "[[rule]]\nname = \"bye\"\non = [\"session-leaving\"]\n[[rule.step]]\nrun = \"go.sh\"\n",
        )
        .unwrap();
        let steps = Arc::new(FakeSteps { ran: Mutex::new(Vec::new()) });
        let scripts = MachineScripts::new(
            home.path().to_path_buf(),
            home.path().join("state"),
            steps.clone(),
            Arc::new(Quiet),
            Arc::new(|| vec!["box".to_owned()]),
            Arc::new(SystemTime::now),
        );
        (home, scripts, steps)
    }

    fn leaving_context() -> Context {
        Context {
            machine: "box".to_owned(),
            trigger: Trigger::SessionLeaving,
            session: Some(SessionContext {
                session: "s1".to_owned(),
                channel: "main".to_owned(),
                folder: "sessions/s1".to_owned(),
                from: "box".to_owned(),
                to: "other".to_owned(),
            }),
        }
    }

    #[tokio::test]
    async fn leaving_rules_have_run_when_the_move_may_go_on() {
        let (_home, scripts, steps) = engine("tidy");
        let in_time = leaving(&scripts, leaving_context(), Duration::from_secs(30)).await;
        assert!(in_time);
        assert_eq!(*lock(&steps.ran), ["tidy"]);
    }

    #[tokio::test(start_paused = true)]
    async fn leaving_rules_that_hang_never_hold_a_move_past_the_cap() {
        let (_home, scripts, steps) = engine("hang");
        let in_time = leaving(&scripts, leaving_context(), Duration::from_secs(5)).await;
        assert!(!in_time);
        assert_eq!(*lock(&steps.ran), ["hang"]);
    }
}
