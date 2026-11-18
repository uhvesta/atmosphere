//! The run engine: which rules a trigger runs, in what order, one run at a
//! time per machine.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use humata2_machines::{
    Keys, MachineScriptsError, ScriptBody, ScriptRule, ScriptRules, ScriptStep as Step, Trigger, Wait, WAIT_IDLE, read_machine_scripts,
    scripts_dir,
};
use tokio::sync::oneshot;
use tokio::time::Instant;

use super::log::RunLog;
use super::{Context, LogError, Outcome, Report, RunRecord, ShellOutput, StepError, StepRun, Steps};

/// Longest step label kept in a record.
const LABEL_CHARS: usize = 80;

#[derive(Debug, thiserror::Error)]
pub(crate) enum RunNowError {
    #[error("the machine scripts file can't be used: {source}. Fix the problems it lists, or run `humata machine script check`")]
    Rules {
        #[source]
        source: MachineScriptsError,
    },
    #[error("there is no rule named {rule:?} in the machine scripts file. Check the name with `humata machine script list`")]
    NoRule { rule: String },
    #[error("the rule {rule:?} isn't for {machine}: its `machines` list leaves this machine out. Add the machine to the list, or run a rule that applies to it")]
    NotForMachine { rule: String, machine: String },
}

/// What a machine has waiting behind its running work.
#[derive(Default)]
struct Lane {
    /// A drain task is going for this machine.
    draining: bool,
    queue: VecDeque<Queued>,
    /// Held while a run for this machine is going.
    exec: Arc<tokio::sync::Mutex<()>>,
}

struct Queued {
    context: Context,
    waiters: Vec<oneshot::Sender<Vec<RunRecord>>>,
}

pub(crate) struct MachineScripts {
    humata_home: PathBuf,
    steps: Arc<dyn Steps>,
    report: Arc<dyn Report>,
    machines: Arc<dyn Fn() -> Vec<String> + Send + Sync>,
    clock: Arc<dyn Fn() -> SystemTime + Send + Sync>,
    log: RunLog,
    lanes: Mutex<HashMap<String, Lane>>,
    counter: AtomicU64,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn millis(time: SystemTime) -> i64 {
    time.duration_since(UNIX_EPOCH)
        .map(|since| i64::try_from(since.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

fn took(since: Instant) -> u64 {
    u64::try_from(since.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn short(text: &str) -> String {
    text.chars().take(LABEL_CHARS).collect()
}

fn describe(step: &Step) -> (&'static str, String) {
    match step {
        Step::Run { script, args } => ("run", short(&[script.as_str().to_string()].into_iter().chain(args.iter().cloned()).collect::<Vec<_>>().join(" "))),
        Step::Humata { args } => ("humata", short(&args.join(" "))),
        Step::Start { agent, channel, .. } => ("start", short(&format!("{agent} in {channel}"))),
        Step::Send { text, .. } => ("send", short(text)),
    }
}

/// The variables a script and CLI step see (D685).
fn environment(context: &Context) -> Vec<(String, String)> {
    let mut env = vec![
        ("HUMATA_TRIGGER".to_string(), context.trigger.as_str().to_string()),
        ("HUMATA_MACHINE".to_string(), context.machine.clone()),
    ];
    if let Some(session) = &context.session {
        for (name, value) in [
            ("HUMATA_SESSION", &session.session),
            ("HUMATA_CHANNEL", &session.channel),
            ("HUMATA_SESSION_DIR", &session.folder),
            ("HUMATA_FROM", &session.from),
            ("HUMATA_TO", &session.to),
        ] {
            if !value.is_empty() {
                env.push((name.to_string(), value.clone()));
            }
        }
    }
    env
}

/// What a step's work came to when it ran to its end.
struct Done {
    outcome: Outcome,
    exit: Option<i32>,
    tail: Vec<String>,
    error: Option<String>,
}

impl Done {
    fn passed() -> Self {
        Done { outcome: Outcome::Passed, exit: None, tail: Vec::new(), error: None }
    }
}

fn shell_done(output: ShellOutput) -> Done {
    let (outcome, error) = if output.timed_out {
        (Outcome::TimedOut, Some("the step ran past its time limit and was stopped".to_string()))
    } else {
        match output.exit {
            Some(0) => (Outcome::Passed, None),
            Some(code) => (Outcome::Failed, Some(format!("exited with {code}"))),
            None => (Outcome::Failed, Some("ended by a signal".to_string())),
        }
    };
    Done { outcome, exit: output.exit, tail: output.tail, error }
}

impl MachineScripts {
    pub(crate) fn new(
        humata_home: PathBuf,
        state_dir: PathBuf,
        steps: Arc<dyn Steps>,
        report: Arc<dyn Report>,
        machines: Arc<dyn Fn() -> Vec<String> + Send + Sync>,
        clock: Arc<dyn Fn() -> SystemTime + Send + Sync>,
    ) -> Arc<Self> {
        Arc::new(MachineScripts {
            humata_home,
            steps,
            report,
            machines,
            clock,
            log: RunLog::new(&state_dir),
            lanes: Mutex::new(HashMap::new()),
            counter: AtomicU64::new(0),
        })
    }

    /// Runs what `context` fires, in the background. Returns at once.
    pub(crate) fn fire(self: &Arc<Self>, context: Context) {
        drop(self.enqueue(context));
    }

    /// Runs what `context` fires and returns the runs made. A run that was
    /// folded into one already waiting returns that one's records.
    pub(crate) async fn fire_and_wait(self: &Arc<Self>, context: Context) -> Vec<RunRecord> {
        self.enqueue(context).await.unwrap_or_default()
    }

    /// Runs `rule` now on the context's machine, whether or not it has
    /// passed before when it is `once`; a pass is recorded for `once`.
    pub(crate) async fn run_now(self: &Arc<Self>, rule: &str, context: Context) -> Result<RunRecord, RunNowError> {
        let rules = self.rules().map_err(|source| RunNowError::Rules { source })?;
        let found = rules
            .rules
            .iter()
            .find(|candidate| candidate.name == rule)
            .ok_or_else(|| RunNowError::NoRule { rule: rule.to_string() })?;
        if !found.scope.includes(&context.machine) {
            return Err(RunNowError::NotForMachine { rule: rule.to_string(), machine: context.machine });
        }
        let exec = lock(&self.lanes).entry(context.machine.clone()).or_default().exec.clone();
        let _running = exec.lock().await;
        let record = self.execute(found, &context).await;
        self.finish(found, &record).await;
        Ok(record)
    }

    /// The machine's runs, newest first.
    pub(crate) fn runs(&self, machine: &str, rule: Option<&str>, limit: usize) -> Result<Vec<RunRecord>, LogError> {
        self.log.read(machine, rule, limit)
    }

    /// The rules file as it is now.
    pub(crate) fn rules(&self) -> Result<ScriptRules, MachineScriptsError> {
        read_machine_scripts(&self.humata_home, &(self.machines)())
    }

    /// Queues `context` behind this machine's running work. Repeated
    /// `connected` fires waiting together are one run, and so are
    /// repeated `upgraded` ones (D689).
    fn enqueue(self: &Arc<Self>, context: Context) -> oneshot::Receiver<Vec<RunRecord>> {
        let (done, wait) = oneshot::channel();
        let machine = context.machine.clone();
        let start = {
            let mut lanes = lock(&self.lanes);
            let lane = lanes.entry(machine.clone()).or_default();
            let folded = if matches!(context.trigger, Trigger::Connected | Trigger::Upgraded) {
                lane.queue.iter().position(|queued| queued.context.trigger == context.trigger)
            } else {
                None
            };
            match folded {
                Some(at) => lane.queue[at].waiters.push(done),
                None => lane.queue.push_back(Queued { context, waiters: vec![done] }),
            }
            !std::mem::replace(&mut lane.draining, true)
        };
        if start {
            let engine = Arc::clone(self);
            tokio::spawn(async move { engine.drain(machine).await });
        }
        wait
    }

    async fn drain(self: Arc<Self>, machine: String) {
        loop {
            let next = {
                let mut lanes = lock(&self.lanes);
                let Some(lane) = lanes.get_mut(&machine) else { return };
                match lane.queue.pop_front() {
                    Some(queued) => (queued, lane.exec.clone()),
                    None => {
                        lane.draining = false;
                        return;
                    }
                }
            };
            let (queued, exec) = next;
            let records = {
                let _running = exec.lock().await;
                self.run_context(&queued.context).await
            };
            for waiter in queued.waiters {
                // A fire nobody waits for dropped its end.
                let _ = waiter.send(records.clone());
            }
        }
    }

    /// Every rule `context` fires, in file order.
    async fn run_context(&self, context: &Context) -> Vec<RunRecord> {
        let rules = match self.rules() {
            Ok(rules) => rules,
            Err(error) => {
                crate::log::line(&format!(
                    "humatad2: machine scripts didn't run for {} ({}): {error}",
                    context.machine,
                    context.trigger.as_str()
                ));
                return Vec::new();
            }
        };
        let mut records = Vec::new();
        for rule in rules.rules.iter().filter(|rule| rule.applies(context.trigger, &context.machine)) {
            if rule.once {
                match self.log.once_passed(&context.machine, &rule.name) {
                    Ok(true) => continue,
                    Ok(false) => {}
                    Err(error) => crate::log::line(&format!(
                        "humatad2: can't tell whether machine script {} already passed on {}, so it runs: {error}",
                        rule.name, context.machine
                    )),
                }
            }
            let record = self.execute(rule, context).await;
            self.finish(rule, &record).await;
            records.push(record);
        }
        records
    }

    /// Logs the run, then tells the Inbox and the feed.
    async fn finish(&self, rule: &ScriptRule, record: &RunRecord) {
        if let Err(error) = self.log.append(record) {
            crate::log::line(&format!("humatad2: the machine script run of {} on {} isn't in the log: {error}", record.rule, record.machine));
        }
        if record.outcome == Outcome::Passed {
            if rule.once {
                if let Err(error) = self.log.mark_once(&record.machine, &rule.name) {
                    crate::log::line(&format!(
                        "humatad2: machine script {} passed on {} but can't be marked done, so it will run again: {error}",
                        rule.name, record.machine
                    ));
                }
            }
            self.report.passed(record).await;
        } else {
            self.report.failed(record).await;
        }
        self.report.changed(&record.machine);
    }

    /// Runs the steps of `rule`; the first that doesn't pass stops it.
    async fn execute(&self, rule: &ScriptRule, context: &Context) -> RunRecord {
        let begun = Instant::now();
        let started_at_ms = millis((self.clock)());
        let id = format!("{started_at_ms}-{}", self.counter.fetch_add(1, Ordering::Relaxed));
        let env = environment(context);
        let mut started: Option<String> = None;
        let mut outcome = Outcome::Passed;
        let mut steps = Vec::with_capacity(rule.steps.len());
        for (at, step) in rule.steps.iter().enumerate() {
            let index = u32::try_from(at).unwrap_or(u32::MAX);
            if outcome != Outcome::Passed {
                let (kind, label) = describe(step);
                steps.push(StepRun { index, kind, label, outcome: Outcome::Skipped, exit: None, tail: Vec::new(), error: None, took_ms: 0 });
                continue;
            }
            let run = self.run_step(rule, context, &id, index, step, &env, &mut started).await;
            outcome = run.outcome;
            steps.push(run);
        }
        RunRecord {
            id,
            machine: context.machine.clone(),
            rule: rule.name.clone(),
            trigger: context.trigger,
            session: context.session.as_ref().map(|session| session.session.clone()),
            started_at_ms,
            took_ms: took(begun),
            outcome,
            steps,
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_step(
        &self,
        rule: &ScriptRule,
        context: &Context,
        run_id: &str,
        index: u32,
        step: &Step,
        env: &[(String, String)],
        started: &mut Option<String>,
    ) -> StepRun {
        let (kind, label) = describe(step);
        let limit = rule.step_timeout(context.trigger);
        let begun = Instant::now();
        let work = self.step_work(rule, context, run_id, index, step, env, limit, started);
        let done = match tokio::time::timeout(limit, work).await {
            Err(_) => Done {
                outcome: Outcome::TimedOut,
                exit: None,
                tail: Vec::new(),
                error: Some(format!("the step ran past its {} second limit and was stopped", limit.as_secs())),
            },
            Ok(Err(error)) => Done { outcome: Outcome::Failed, exit: None, tail: Vec::new(), error: Some(error.to_string()) },
            Ok(Ok(done)) => done,
        };
        StepRun { index, kind, label, outcome: done.outcome, exit: done.exit, tail: done.tail, error: done.error, took_ms: took(begun) }
    }

    #[allow(clippy::too_many_arguments)]
    async fn step_work(
        &self,
        rule: &ScriptRule,
        context: &Context,
        run_id: &str,
        index: u32,
        step: &Step,
        env: &[(String, String)],
        limit: Duration,
        started: &mut Option<String>,
    ) -> Result<Done, StepError> {
        // Keyed so a retry of this run never does a step twice.
        let key = format!("{run_id}/{index}");
        match step {
            Step::Run { script, args } => {
                let path = scripts_dir(&self.humata_home).join(script.as_str());
                // A small file, read where the run is: no blocking pool, so
                // a paused test clock can't run ahead of it.
                let bytes = std::fs::read(&path).map_err(|source| StepError::ReadScript { path, source })?;
                let output = self
                    .steps
                    .shell(&context.machine, ScriptBody::Bash(Arc::from(bytes)), args.clone(), env.to_vec(), limit)
                    .await?;
                Ok(shell_done(output))
            }
            Step::Humata { args } => {
                let output = self
                    .steps
                    .shell(&context.machine, ScriptBody::Humata(args.clone()), Vec::new(), env.to_vec(), limit)
                    .await?;
                Ok(shell_done(output))
            }
            Step::Start { agent, channel, title } => {
                let title = title.clone().unwrap_or_else(|| rule.name.clone());
                let session = self.steps.start(&context.machine, agent, channel, &title, &key).await?;
                *started = Some(session);
                Ok(Done::passed())
            }
            Step::Send { text, keys, wait } => {
                let session = started
                    .clone()
                    .or_else(|| context.session.as_ref().map(|session| session.session.clone()))
                    .ok_or(StepError::NoSession)?;
                if matches!(wait, Wait::Idle) {
                    self.steps.wait_idle(&session, WAIT_IDLE).await?;
                }
                self.steps.send(&session, text, matches!(keys, Keys::Enter), &key).await?;
                Ok(Done::passed())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::atomic::AtomicUsize;

    use super::super::{Fut, SessionContext};
    use super::*;

    /// Records what the engine asks. A script's text says what it does:
    /// "fail" exits 1, "hang" never ends, "slow" takes a moment; anything
    /// else exits 0.
    #[derive(Default)]
    struct FakeSteps {
        calls: Mutex<Vec<String>>,
        envs: Mutex<Vec<Vec<(String, String)>>>,
        running: AtomicUsize,
        peak: AtomicUsize,
    }

    impl FakeSteps {
        fn calls(&self) -> Vec<String> {
            lock(&self.calls).clone()
        }
        fn note(&self, call: String) {
            lock(&self.calls).push(call);
        }
    }

    impl Steps for FakeSteps {
        fn shell<'a>(
            &'a self,
            machine: &'a str,
            body: ScriptBody,
            args: Vec<String>,
            env: Vec<(String, String)>,
            _timeout: Duration,
        ) -> Fut<'a, Result<ShellOutput, StepError>> {
            Box::pin(async move {
                let text = match &body {
                    ScriptBody::Bash(bytes) => String::from_utf8_lossy(bytes).trim().to_string(),
                    ScriptBody::Humata(args) => format!("humata {}", args.join(" ")),
                };
                self.note(format!("shell:{machine}:{text}:{}", args.join(",")));
                lock(&self.envs).push(env);
                let now = self.running.fetch_add(1, Ordering::SeqCst) + 1;
                self.peak.fetch_max(now, Ordering::SeqCst);
                if text == "hang" {
                    tokio::time::sleep(Duration::from_secs(10 * 60)).await;
                } else if text == "slow" {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                self.running.fetch_sub(1, Ordering::SeqCst);
                let exit = if text == "fail" { 1 } else { 0 };
                Ok(ShellOutput { exit: Some(exit), timed_out: false, tail: vec![format!("out of {text}")] })
            })
        }

        fn start<'a>(&'a self, machine: &'a str, agent: &'a str, channel: &'a str, title: &'a str, key: &'a str) -> Fut<'a, Result<String, StepError>> {
            Box::pin(async move {
                self.note(format!("start:{machine}:{agent}:{channel}:{title}:{key}"));
                Ok("started-1".to_string())
            })
        }

        fn wait_idle<'a>(&'a self, session: &'a str, _timeout: Duration) -> Fut<'a, Result<(), StepError>> {
            Box::pin(async move {
                self.note(format!("idle:{session}"));
                Ok(())
            })
        }

        fn send<'a>(&'a self, session: &'a str, text: &'a str, enter: bool, key: &'a str) -> Fut<'a, Result<(), StepError>> {
            Box::pin(async move {
                self.note(format!("send:{session}:{text}:{enter}:{key}"));
                Ok(())
            })
        }
    }

    #[derive(Default)]
    struct FakeReport {
        events: Mutex<Vec<String>>,
    }

    impl FakeReport {
        fn events(&self) -> Vec<String> {
            lock(&self.events).clone()
        }
    }

    impl Report for FakeReport {
        fn failed<'a>(&'a self, run: &'a RunRecord) -> Fut<'a, ()> {
            Box::pin(async move { lock(&self.events).push(format!("failed:{}", run.rule)) })
        }
        fn passed<'a>(&'a self, run: &'a RunRecord) -> Fut<'a, ()> {
            Box::pin(async move { lock(&self.events).push(format!("passed:{}", run.rule)) })
        }
        fn changed(&self, machine: &str) {
            lock(&self.events).push(format!("changed:{machine}"));
        }
    }

    struct Rig {
        home: tempfile::TempDir,
        engine: Arc<MachineScripts>,
        steps: Arc<FakeSteps>,
        report: Arc<FakeReport>,
    }

    impl Rig {
        fn new(rules: &str, scripts: &[(&str, &str)]) -> Rig {
            let home = tempfile::tempdir().unwrap();
            fs::create_dir_all(home.path().join("config")).unwrap();
            fs::create_dir_all(home.path().join("scripts")).unwrap();
            fs::write(home.path().join("config/machine-scripts.toml"), rules).unwrap();
            for (name, text) in scripts {
                fs::write(home.path().join("scripts").join(name), text).unwrap();
            }
            let steps = Arc::new(FakeSteps::default());
            let report = Arc::new(FakeReport::default());
            let engine = MachineScripts::new(
                home.path().to_path_buf(),
                home.path().join("state"),
                steps.clone(),
                report.clone(),
                Arc::new(|| vec!["box".to_string(), "other".to_string()]),
                Arc::new(SystemTime::now),
            );
            Rig { home, engine, steps, report }
        }

        fn script(&self, name: &str, text: &str) {
            fs::write(self.home.path().join("scripts").join(name), text).unwrap();
        }
    }

    fn connected() -> Context {
        Context { machine: "box".to_string(), trigger: Trigger::Connected, session: None }
    }

    fn arrived() -> Context {
        Context {
            machine: "box".to_string(),
            trigger: Trigger::SessionArrived,
            session: Some(SessionContext {
                session: "s9".to_string(),
                channel: "main".to_string(),
                folder: "/work/x".to_string(),
                from: "old".to_string(),
                to: "box".to_string(),
            }),
        }
    }

    fn rule(name: &str, on: &str, extra: &str, steps: &str) -> String {
        format!("[[rule]]\nname = \"{name}\"\non = [\"{on}\"]\n{extra}\n{steps}\n")
    }

    fn run_step(script: &str) -> String {
        format!("[[rule.step]]\nrun = \"{script}\"\n")
    }

    #[tokio::test]
    async fn rules_and_steps_run_in_file_order() {
        let file = rule("a", "connected", "", &(run_step("one.sh") + &run_step("two.sh"))) + &rule("b", "connected", "", &run_step("three.sh"));
        let rig = Rig::new(&file, &[("one.sh", "one"), ("two.sh", "two"), ("three.sh", "three")]);
        let records = rig.engine.fire_and_wait(connected()).await;
        assert_eq!(records.iter().map(|r| r.rule.as_str()).collect::<Vec<_>>(), ["a", "b"]);
        assert!(records.iter().all(|r| r.outcome == Outcome::Passed));
        assert_eq!(rig.steps.calls(), ["shell:box:one:", "shell:box:two:", "shell:box:three:"]);
    }

    #[tokio::test]
    async fn a_rule_not_for_the_trigger_or_machine_does_not_run() {
        let file = rule("later", "added", "", &run_step("one.sh")) + &rule("elsewhere", "connected", "machines = [\"other\"]", &run_step("one.sh"));
        let rig = Rig::new(&file, &[("one.sh", "one")]);
        assert!(rig.engine.fire_and_wait(connected()).await.is_empty());
        assert!(rig.steps.calls().is_empty());
    }

    #[tokio::test]
    async fn the_first_failed_step_stops_its_rule_and_the_next_rule_runs() {
        let file = rule("a", "connected", "", &(run_step("bad.sh") + &run_step("never.sh"))) + &rule("b", "connected", "", &run_step("fine.sh"));
        let rig = Rig::new(&file, &[("bad.sh", "fail"), ("never.sh", "never"), ("fine.sh", "fine")]);
        let records = rig.engine.fire_and_wait(connected()).await;
        assert_eq!(records[0].outcome, Outcome::Failed);
        assert_eq!(records[0].steps[0].outcome, Outcome::Failed);
        assert_eq!(records[0].steps[0].exit, Some(1));
        assert_eq!(records[0].steps[1].outcome, Outcome::Skipped);
        assert_eq!(records[1].outcome, Outcome::Passed);
        assert_eq!(rig.steps.calls(), ["shell:box:fail:", "shell:box:fine:"]);
    }

    #[tokio::test]
    async fn once_runs_once_and_a_failure_is_retried() {
        let file = rule("setup", "connected", "once = true", &run_step("s.sh"));
        let rig = Rig::new(&file, &[("s.sh", "fail")]);
        assert_eq!(rig.engine.fire_and_wait(connected()).await.len(), 1);
        assert_eq!(rig.engine.fire_and_wait(connected()).await.len(), 1);
        rig.script("s.sh", "fine");
        assert_eq!(rig.engine.fire_and_wait(connected()).await[0].outcome, Outcome::Passed);
        assert!(rig.engine.fire_and_wait(connected()).await.is_empty());
        assert_eq!(rig.steps.calls().len(), 3);
    }

    #[tokio::test]
    async fn send_types_into_the_session_a_start_step_made() {
        let steps = "[[rule.step]]\nstart = { agent = \"copilot\", channel = \"main\" }\n[[rule.step]]\nsend = \"/restart\"\nwait = \"none\"\n";
        let rig = Rig::new(&rule("r", "connected", "", steps), &[]);
        let records = rig.engine.fire_and_wait(connected()).await;
        assert_eq!(records[0].outcome, Outcome::Passed);
        let calls = rig.steps.calls();
        // The title is the rule's name, and the keys are per run and step.
        assert!(calls[0].starts_with("start:box:copilot:main:r:"));
        assert!(calls[1].starts_with("send:started-1:/restart:true:"));
        assert!(calls[0].ends_with("/0"));
        assert!(calls[1].ends_with("/1"));
    }

    #[tokio::test]
    async fn send_types_into_the_moved_session_on_a_session_trigger() {
        let steps = "[[rule.step]]\nsend = \"/go\"\nkeys = \"none\"\nwait = \"none\"\n";
        let rig = Rig::new(&rule("r", "session-arrived", "", steps), &[]);
        let records = rig.engine.fire_and_wait(arrived()).await;
        assert_eq!(records[0].session.as_deref(), Some("s9"));
        assert!(rig.steps.calls()[0].starts_with("send:s9:/go:false:"));
    }

    #[tokio::test]
    async fn send_with_no_session_fails() {
        let rig = Rig::new(&rule("r", "connected", "", "[[rule.step]]\nsend = \"/go\"\nwait = \"none\"\n"), &[]);
        let records = rig.engine.fire_and_wait(connected()).await;
        assert_eq!(records[0].outcome, Outcome::Failed);
        assert!(rig.steps.calls().is_empty());
        assert!(records[0].steps[0].error.is_some());
    }

    #[tokio::test]
    async fn the_wait_for_idle_comes_before_a_send_that_asks_for_it() {
        let steps = "[[rule.step]]\nsend = \"a\"\nwait = \"idle\"\n[[rule.step]]\nsend = \"b\"\nwait = \"none\"\n";
        let rig = Rig::new(&rule("r", "session-arrived", "", steps), &[]);
        rig.engine.fire_and_wait(arrived()).await;
        let calls = rig.steps.calls();
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[0], "idle:s9");
        assert!(calls[1].starts_with("send:s9:a:"));
        assert!(calls[2].starts_with("send:s9:b:"));
    }

    #[tokio::test(start_paused = true)]
    async fn a_step_past_its_limit_is_timed_out() {
        let file = rule("r", "connected", "", &(run_step("h.sh") + &run_step("after.sh")));
        let rig = Rig::new(&file, &[("h.sh", "hang"), ("after.sh", "after")]);
        let records = rig.engine.fire_and_wait(connected()).await;
        assert_eq!(records[0].outcome, Outcome::TimedOut);
        assert_eq!(records[0].steps[0].outcome, Outcome::TimedOut);
        assert_eq!(records[0].steps[1].outcome, Outcome::Skipped);
        assert_eq!(rig.report.events()[0], "failed:r");
    }

    #[tokio::test]
    async fn session_triggers_pass_the_session_in_the_environment() {
        let rig = Rig::new(&rule("r", "session-arrived", "", &run_step("e.sh")), &[("e.sh", "env")]);
        rig.engine.fire_and_wait(arrived()).await;
        let env = lock(&rig.steps.envs)[0].clone();
        let get = |name: &str| env.iter().find(|(key, _)| key == name).map(|(_, value)| value.as_str());
        assert_eq!(get("HUMATA_TRIGGER"), Some("session-arrived"));
        assert_eq!(get("HUMATA_MACHINE"), Some("box"));
        assert_eq!(get("HUMATA_SESSION"), Some("s9"));
        assert_eq!(get("HUMATA_CHANNEL"), Some("main"));
        assert_eq!(get("HUMATA_SESSION_DIR"), Some("/work/x"));
        assert_eq!(get("HUMATA_FROM"), Some("old"));
        assert_eq!(get("HUMATA_TO"), Some("box"));
    }

    #[tokio::test]
    async fn a_machine_trigger_has_no_session_variables() {
        let rig = Rig::new(&rule("r", "connected", "", &run_step("e.sh")), &[("e.sh", "env")]);
        rig.engine.fire_and_wait(connected()).await;
        let env = lock(&rig.steps.envs)[0].clone();
        assert!(env.iter().all(|(key, _)| key == "HUMATA_TRIGGER" || key == "HUMATA_MACHINE"));
    }

    #[tokio::test]
    async fn a_failure_is_reported_and_a_later_pass_withdraws_it() {
        let rig = Rig::new(&rule("r", "connected", "", &run_step("s.sh")), &[("s.sh", "fail")]);
        rig.engine.fire_and_wait(connected()).await;
        rig.script("s.sh", "fine");
        rig.engine.fire_and_wait(connected()).await;
        assert_eq!(rig.report.events(), ["failed:r", "changed:box", "passed:r", "changed:box"]);
        let logged = rig.engine.runs("box", None, 10).unwrap();
        assert_eq!(logged.len(), 2);
        assert_eq!(logged[0].outcome, Outcome::Passed);
        assert_eq!(logged[1].outcome, Outcome::Failed);
        assert_eq!(logged[1].steps[0].tail, ["out of fail"]);
    }

    #[tokio::test]
    async fn fires_for_one_machine_never_overlap_and_waiting_connects_fold_into_one() {
        let rig = Rig::new(&rule("r", "connected", "", &run_step("s.sh")), &[("s.sh", "slow")]);
        rig.engine.fire(connected());
        let mut spins = 0;
        while rig.steps.running.load(Ordering::SeqCst) == 0 {
            spins += 1;
            assert!(spins < 10_000, "the first run never started");
            tokio::task::yield_now().await;
        }
        let (second, third) = tokio::join!(rig.engine.fire_and_wait(connected()), rig.engine.fire_and_wait(connected()));
        assert_eq!(second.len(), 1);
        assert_eq!(third.len(), 1);
        assert_eq!(second[0].id, third[0].id);
        assert_eq!(rig.steps.calls().len(), 2);
        assert_eq!(rig.steps.peak.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn different_machines_do_not_wait_for_each_other() {
        let rig = Rig::new(&rule("r", "connected", "", &run_step("s.sh")), &[("s.sh", "slow")]);
        let elsewhere = Context { machine: "other".to_string(), ..connected() };
        tokio::join!(rig.engine.fire_and_wait(connected()), rig.engine.fire_and_wait(elsewhere));
        assert_eq!(rig.steps.peak.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_rules_file_that_does_not_parse_runs_nothing() {
        let rig = Rig::new("this is [not toml", &[]);
        assert!(rig.engine.fire_and_wait(connected()).await.is_empty());
        assert!(rig.engine.runs("box", None, 10).unwrap().is_empty());
    }

    #[tokio::test]
    async fn run_now_names_the_rule_and_the_machine_it_is_wrong_for() {
        let file = rule("r", "connected", "machines = [\"other\"]", &run_step("s.sh"));
        let rig = Rig::new(&file, &[("s.sh", "fine")]);
        assert!(matches!(rig.engine.run_now("nope", connected()).await, Err(RunNowError::NoRule { .. })));
        assert!(matches!(rig.engine.run_now("r", connected()).await, Err(RunNowError::NotForMachine { .. })));
        let ok = Context { machine: "other".to_string(), ..connected() };
        assert_eq!(rig.engine.run_now("r", ok).await.unwrap().outcome, Outcome::Passed);
    }

    #[tokio::test]
    async fn run_now_ignores_once_and_records_the_pass() {
        let file = rule("setup", "connected", "once = true", &run_step("s.sh"));
        let rig = Rig::new(&file, &[("s.sh", "fine")]);
        rig.engine.fire_and_wait(connected()).await;
        assert!(rig.engine.fire_and_wait(connected()).await.is_empty());
        assert_eq!(rig.engine.run_now("setup", connected()).await.unwrap().outcome, Outcome::Passed);
        assert_eq!(rig.steps.calls().len(), 2);
    }

    #[tokio::test]
    async fn run_now_of_a_once_rule_that_failed_does_not_count_as_a_pass() {
        let file = rule("setup", "connected", "once = true", &run_step("s.sh"));
        let rig = Rig::new(&file, &[("s.sh", "fail")]);
        assert_eq!(rig.engine.run_now("setup", connected()).await.unwrap().outcome, Outcome::Failed);
        rig.script("s.sh", "fine");
        assert_eq!(rig.engine.fire_and_wait(connected()).await.len(), 1);
    }
}
