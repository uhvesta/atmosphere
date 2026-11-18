//! Machine scripts: the rules file and the command that runs a script on a
//! machine (MACHINESCRIPTS).

use std::sync::Arc;
use std::time::Duration;

use humata2_machines::{
    parse_machine_scripts, read_machine_scripts, rules_path, scripts_dir, Machines, MachinesConfig, ManageError,
    MachineScriptsError, Purpose, RetryPolicy, ScriptBody, ScriptName, ScriptProblem, ScriptRule, ScriptStep, ScriptWarning,
    Scope, Trigger, Keys, Wait, SAMPLE_RULES,
};
use tempfile::TempDir;

const MACHINES: [&str; 2] = ["devbox", "box2"];

fn machines() -> Vec<String> {
    MACHINES.iter().map(|name| (*name).to_owned()).collect()
}

fn present(_: &ScriptName) -> bool {
    true
}

fn problems_with(text: &str, exists: &dyn Fn(&ScriptName) -> bool) -> Vec<ScriptProblem> {
    match parse_machine_scripts(text, exists, &machines()) {
        Err(MachineScriptsError::Invalid { problems }) => problems,
        other => panic!("expected problems, got {other:?}"),
    }
}

fn problems(text: &str) -> Vec<ScriptProblem> {
    problems_with(text, &present)
}

const VALID: &str = r#"
[[rule]]
name = "set-up"
on = ["added", "connected"]
machines = ["devbox"]
once = true
timeout = 600

[[rule.step]]
run = "setup.sh"
args = ["--quiet", "x y"]

[[rule.step]]
humata = ["session", "list"]

[[rule.step]]
start = { agent = "copilot", channel = "tools", title = "Tools" }

[[rule.step]]
send = "/restart"
keys = "none"
wait = "none"

[[rule]]
name = "after-move"
on = "session-arrived"

[[rule.step]]
send = "hello"
"#;

fn valid() -> Vec<ScriptRule> {
    parse_machine_scripts(VALID, &present, &machines()).unwrap().rules
}

#[test]
fn a_valid_file_parses_to_its_rules_and_steps() {
    let rules = valid();
    assert_eq!(rules.len(), 2);
    let first = &rules[0];
    assert_eq!(first.name, "set-up");
    assert_eq!(first.on, [Trigger::Added, Trigger::Connected]);
    assert_eq!(first.scope, Scope::Named(vec!["devbox".to_owned()]));
    assert!(first.once);
    assert_eq!(first.timeout, Duration::from_secs(600));
    assert_eq!(first.steps.len(), 4);
    match &first.steps[0] {
        ScriptStep::Run { script, args } => {
            assert_eq!(script.as_str(), "setup.sh");
            assert_eq!(args, &["--quiet", "x y"]);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(first.steps[1], ScriptStep::Humata { args: vec!["session".into(), "list".into()] });
    assert_eq!(
        first.steps[2],
        ScriptStep::Start { agent: "copilot".into(), channel: "tools".into(), title: Some("Tools".into()) }
    );
    assert_eq!(first.steps[3], ScriptStep::Send { text: "/restart".into(), keys: Keys::None, wait: Wait::None });

    let second = &rules[1];
    assert_eq!(second.on, [Trigger::SessionArrived]);
    assert_eq!(second.scope, Scope::All);
    assert!(!second.once);
    assert_eq!(second.timeout, Duration::from_secs(3600));
    assert_eq!(second.steps, [ScriptStep::Send { text: "hello".into(), keys: Keys::Enter, wait: Wait::Idle }]);
}

#[test]
fn each_kind_of_problem_has_its_variant() {
    let rule = |body: &str| format!("[[rule]]\nname = \"r\"\n{body}");
    let step = |body: &str| rule(&format!("on = \"connected\"\n[[rule.step]]\n{body}\n"));
    let session_step = |body: &str| rule(&format!("on = \"session-arrived\"\n[[rule.step]]\n{body}\n"));

    assert!(matches!(problems("[[rule]]\nname = \"Bad Name\"\non = \"added\"\n[[rule.step]]\nsend = \"x\"\n").as_slice(),
        [ScriptProblem::BadName { .. }]));
    let twice = format!("{0}{0}", step("send = \"x\""));
    assert!(matches!(problems(&twice).as_slice(), [ScriptProblem::DuplicateName { rule }] if rule == "r"));
    assert!(matches!(problems(&rule("[[rule.step]]\nsend = \"x\"\n")).as_slice(), [ScriptProblem::NoTrigger { .. }]));
    assert!(matches!(
        problems(&rule("on = \"sometimes\"\n[[rule.step]]\nsend = \"x\"\n")).as_slice(),
        [ScriptProblem::UnknownTrigger { trigger, .. }] if trigger == "sometimes"
    ));
    assert!(matches!(problems(&rule("on = \"added\"\n")).as_slice(), [ScriptProblem::NoSteps { .. }]));
    assert!(matches!(problems(&step("")).as_slice(), [ScriptProblem::StepKind { found: 0, step: 1, .. }]));
    assert!(matches!(
        problems(&step("run = \"a.sh\"\nsend = \"x\"")).as_slice(),
        [ScriptProblem::StepKind { found: 2, .. }]
    ));
    assert!(matches!(problems(&step("run = \"../x.sh\"")).as_slice(), [ScriptProblem::BadScriptName { .. }]));
    assert!(matches!(problems(&step("run = \"sub/x.sh\"")).as_slice(), [ScriptProblem::BadScriptName { .. }]));
    assert!(matches!(
        problems_with(&step("run = \"x.sh\""), &|_| false).as_slice(),
        [ScriptProblem::MissingScript { script, .. }] if script == "x.sh"
    ));
    assert!(matches!(problems(&step("humata = []")).as_slice(), [ScriptProblem::EmptyHumata { .. }]));
    assert!(matches!(
        problems(&session_step("start = { agent = \"copilot\", channel = \"c\" }")).as_slice(),
        [ScriptProblem::StartOnSessionTrigger { .. }]
    ));
    assert!(matches!(problems(&step("send = \"  \"")).as_slice(), [ScriptProblem::EmptyField { field: "send", .. }]));
    assert!(matches!(
        problems(&step("send = \"x\"\nkeys = \"tab\"")).as_slice(),
        [ScriptProblem::BadChoice { field: "keys", .. }]
    ));
    assert!(matches!(
        problems(&step("send = \"x\"\nwait = \"soon\"")).as_slice(),
        [ScriptProblem::BadChoice { field: "wait", .. }]
    ));
    assert!(matches!(problems(&step("run = \"a.sh\"\nkeys = \"none\"")).as_slice(), [ScriptProblem::StrayField { .. }]));
    for seconds in ["0", "3601", "-5"] {
        let body = format!("on = \"added\"\ntimeout = {seconds}\n[[rule.step]]\nsend = \"x\"\n");
        assert!(matches!(problems(&rule(&body)).as_slice(), [ScriptProblem::BadTimeout { .. }]), "{seconds}");
    }
}

#[test]
fn every_problem_is_reported_at_once() {
    let found = problems(
        r#"
[[rule]]
name = "BAD"
on = "never"
timeout = 0

[[rule.step]]
run = "a/b.sh"

[[rule]]
name = "ok-name"
on = "connected"
"#,
    );
    let rule_of = |p: &ScriptProblem| match p {
        ScriptProblem::BadName { rule }
        | ScriptProblem::UnknownTrigger { rule, .. }
        | ScriptProblem::BadTimeout { rule, .. }
        | ScriptProblem::BadScriptName { rule, .. }
        | ScriptProblem::NoSteps { rule } => rule.clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!(found.len(), 5);
    assert_eq!(found.iter().filter(|p| rule_of(p) == "BAD").count(), 4);
    assert_eq!(found.iter().filter(|p| rule_of(p) == "ok-name").count(), 1);
}

#[test]
fn an_unknown_machine_is_only_a_warning_and_star_is_everything() {
    let rules = parse_machine_scripts(
        "[[rule]]\nname = \"r\"\non = \"added\"\nmachines = [\"later\", \"devbox\"]\n[[rule.step]]\nsend = \"x\"\n\
         [[rule]]\nname = \"s\"\non = \"added\"\nmachines = [\"*\", \"nope\"]\n[[rule.step]]\nsend = \"x\"\n",
        &present,
        &machines(),
    )
    .unwrap();
    assert_eq!(rules.warnings, [ScriptWarning::UnknownMachine { rule: "r".into(), machine: "later".into() }]);
    assert_eq!(rules.rules[1].scope, Scope::All);
}

#[test]
fn unknown_keys_and_bad_toml_are_syntax_errors() {
    for text in ["[[rule]]\nname = \"r\"\nnope = 1\n", "[[rule]\n", "[extra]\nx = 1\n"] {
        assert!(matches!(
            parse_machine_scripts(text, &present, &[]),
            Err(MachineScriptsError::Syntax { .. })
        ));
    }
}

#[test]
fn a_missing_file_is_no_rules_and_a_present_one_is_checked_against_the_scripts_folder() {
    let home = TempDir::new().unwrap();
    let none = read_machine_scripts(home.path(), &[]).unwrap();
    assert!(none.rules.is_empty() && none.warnings.is_empty());

    std::fs::create_dir_all(rules_path(home.path()).parent().unwrap()).unwrap();
    std::fs::write(
        rules_path(home.path()),
        "[[rule]]\nname = \"r\"\non = \"added\"\n[[rule.step]]\nrun = \"setup.sh\"\n",
    )
    .unwrap();
    assert!(matches!(
        read_machine_scripts(home.path(), &[]),
        Err(MachineScriptsError::Invalid { problems }) if matches!(problems.as_slice(), [ScriptProblem::MissingScript { .. }])
    ));

    std::fs::create_dir_all(scripts_dir(home.path())).unwrap();
    std::fs::write(scripts_dir(home.path()).join("setup.sh"), "true\n").unwrap();
    assert_eq!(read_machine_scripts(home.path(), &[]).unwrap().rules.len(), 1);
}

#[test]
fn the_sample_file_is_all_comments_and_parses_to_no_rules() {
    assert!(SAMPLE_RULES.lines().all(|line| line.is_empty() || line.starts_with('#')));
    let rules = parse_machine_scripts(SAMPLE_RULES, &present, &machines()).unwrap();
    assert!(rules.rules.is_empty());
}

#[test]
fn scope_and_triggers_decide_which_rules_apply() {
    let rules = valid();
    let (set_up, after_move) = (&rules[0], &rules[1]);
    assert!(set_up.applies(Trigger::Added, "devbox"));
    assert!(set_up.applies(Trigger::Connected, "devbox"));
    assert!(!set_up.applies(Trigger::Connected, "box2"));
    assert!(!set_up.applies(Trigger::SessionArrived, "devbox"));
    assert!(after_move.applies(Trigger::SessionArrived, "box2"));
    assert!(!after_move.applies(Trigger::Added, "box2"));
    assert!(set_up.applies(Trigger::Manual, "devbox"));
    assert!(!set_up.applies(Trigger::Manual, "box2"));
    assert!(Scope::All.includes("anything"));
}

#[test]
fn step_timeouts_are_capped() {
    let rules = valid();
    let (short, long) = (&rules[0], &rules[1]); // 600 s and 3600 s caps
    assert_eq!(long.step_timeout(Trigger::Connected), Duration::from_secs(300));
    assert_eq!(long.step_timeout(Trigger::SessionLeaving), Duration::from_secs(60));
    assert_eq!(short.step_timeout(Trigger::Added), Duration::from_secs(300));

    let tight = parse_machine_scripts(
        "[[rule]]\nname = \"t\"\non = \"added\"\ntimeout = 20\n[[rule.step]]\nsend = \"x\"\n",
        &present,
        &[],
    )
    .unwrap();
    assert_eq!(tight.rules[0].step_timeout(Trigger::Added), Duration::from_secs(20));
    assert_eq!(tight.rules[0].step_timeout(Trigger::SessionLeaving), Duration::from_secs(20));
}

#[test]
fn trigger_names_round_trip() {
    for trigger in [Trigger::Added, Trigger::Connected, Trigger::SessionArrived, Trigger::SessionLeaving, Trigger::Manual] {
        assert_eq!(Trigger::parse(trigger.as_str()), Some(trigger));
    }
    assert_eq!(Trigger::parse("later"), None);
    assert!(Trigger::SessionArrived.is_session() && !Trigger::Connected.is_session());
}

fn machines_with_devbox(dir: &TempDir) -> Machines {
    let hosts = dir.path().join("hosts.json");
    std::fs::write(
        &hosts,
        r#"{"version":1,"machines":[{"name":"devbox","ssh_host":"me@box.internal","root":"~/Humata","state":"ready"}]}"#,
    )
    .unwrap();
    Machines::open(MachinesConfig {
        hosts_file: hosts,
        known_hosts: dir.path().join("known_hosts"),
        ssh_config: dir.path().join("ssh_config"),
        peer_keys: dir.path().join("peer-keys"),
        retry: RetryPolicy::default(),
    })
}

#[test]
fn a_bash_script_goes_to_the_machine_on_stdin() {
    let dir = TempDir::new().unwrap();
    let machines = machines_with_devbox(&dir);
    let body: Arc<[u8]> = Arc::from(&b"echo hi\n"[..]);
    let invocation = machines
        .machine_script(
            "devbox",
            &ScriptBody::Bash(Arc::clone(&body)),
            &["a b".to_owned()],
            &[("HUMATA_MACHINE".to_owned(), "devbox".to_owned())],
            Duration::from_secs(30),
        )
        .unwrap();
    assert_eq!(invocation.stdin.as_deref(), Some(&*body));
    assert_eq!(invocation.purpose, Purpose::MachineScript);
    assert_eq!(invocation.timeout, Some(Duration::from_secs(30)));
}

#[test]
fn a_humata_command_sends_no_stdin() {
    let dir = TempDir::new().unwrap();
    let machines = machines_with_devbox(&dir);
    let invocation = machines
        .machine_script("devbox", &ScriptBody::Humata(vec!["session".into(), "list".into()]), &[], &[], Duration::from_secs(5))
        .unwrap();
    assert!(invocation.stdin.is_none());
    assert_eq!(invocation.purpose, Purpose::MachineScript);
}

#[test]
fn a_bad_variable_name_or_machine_is_refused() {
    let dir = TempDir::new().unwrap();
    let machines = machines_with_devbox(&dir);
    let body = ScriptBody::Humata(vec!["x".into()]);
    for name in ["", "lower", "1ABC", "A-B", "A B", "A=B"] {
        let result =
            machines.machine_script("devbox", &body, &[], &[(name.to_owned(), "v".to_owned())], Duration::from_secs(5));
        assert!(matches!(result, Err(ManageError::BadEnvName { .. })), "{name:?}");
    }
    let unknown = machines.machine_script("nowhere", &body, &[], &[], Duration::from_secs(5));
    assert!(matches!(unknown, Err(ManageError::NotFound { .. })));
}
