//! Providers files: what is refused and where, and what a provider's
//! commands receive when they run.

use std::collections::BTreeMap;

use humata2_machines::{
    parse_status, Action, ConfigError, Exit, HostVariables, MachineName, MachineStatus,
    ProblemKind, Provider, Reach, RunError, Runner, SystemRunner, ValuesError,
};

fn problems(text: &str) -> Vec<(String, ProblemKind)> {
    match Provider::parse(text) {
        Err(ConfigError::Problems(problems)) => problems
            .0
            .into_iter()
            .map(|problem| (problem.field, problem.kind))
            .collect(),
        other => panic!("expected problems, got {other:?}"),
    }
}

fn name(value: &str) -> MachineName {
    MachineName::new(value).unwrap()
}

struct Variables(BTreeMap<&'static str, &'static str>);

impl HostVariables for Variables {
    fn value(&self, name: &str) -> Option<String> {
        self.0.get(name).map(|value| (*value).to_owned())
    }
}

const CLOUD: &str = r#"
[providers.cloud]
label = "Cloud"
params = [
  { name = "size", prompt = "Size", default = "small" },
  { name = "region", prompt = "Region" },
]
env = ["CLOUD_TOKEN"]
create = '''printf '%s|%s|%s|%s|%s\n' {name} "$HUMATA_NAME" "$HUMATA_SIZE" "$HUMATA_REGION" "${#CLOUD_TOKEN}"; printf 'token %s\n' "$CLOUD_TOKEN" >&2'''
connect = "cloudctl ssh {name} -- {cmd}"
status = "cloudctl status {name}"
"#;

fn cloud() -> Provider {
    Provider::parse(CLOUD).unwrap().pop().unwrap()
}

#[test]
fn every_problem_in_a_provider_is_reported_at_once() {
    let found = problems(
        r#"
[providers.broken]
start = "echo {cmd}"
params = [{ name = "name" }, { name = "Size" }, { name = "disk" }, { name = "disk" }]
env = ["PATH", "lower", "TOKEN", "TOKEN"]
"#,
    );

    let kinds: Vec<&ProblemKind> = found.iter().map(|(_, kind)| kind).collect();
    for wanted in [
        ProblemKind::MissingCreate,
        ProblemKind::MissingLink,
        ProblemKind::CmdOutsideConnect,
        ProblemKind::ReservedParam,
        ProblemKind::BadParamName { name: "Size".to_owned() },
        ProblemKind::DuplicateParam { name: "disk".to_owned() },
        ProblemKind::ReservedEnvName { name: "PATH".to_owned() },
        ProblemKind::BadEnvName { name: "lower".to_owned() },
        ProblemKind::DuplicateEnv { name: "TOKEN".to_owned() },
    ] {
        assert!(kinds.contains(&&wanted), "{wanted:?} in {found:?}");
    }
}

#[test]
fn a_link_must_be_connect_with_cmd_or_identity_but_not_both() {
    let without_cmd = problems("[providers.a]\ncreate = \"true\"\nconnect = \"ssh {name}\"\n");
    let both = problems(
        "[providers.a]\ncreate = \"true\"\nconnect = \"ssh {name} {cmd}\"\nidentity = \"cat id.json\"\n",
    );

    assert!(without_cmd
        .iter()
        .any(|(field, kind)| field == "connect" && *kind == ProblemKind::ConnectWithoutCmd));
    assert!(both.iter().any(|(_, kind)| *kind == ProblemKind::BothLinks));
}

#[test]
fn a_problem_points_at_its_line() {
    let text = "[providers.a]\ncreate = \"true\"\n\nconnect = \"ssh {name}\"\n";

    let Err(ConfigError::Problems(problems)) = Provider::parse(text) else {
        panic!("expected problems");
    };

    assert_eq!(problems.0[0].line, Some(4));
}

#[test]
fn an_unknown_field_is_a_syntax_error_at_its_line() {
    let text = "[providers.a]\ncreate = \"true\"\nconnect = \"x {cmd}\"\ncreat = \"typo\"\n";

    let Err(ConfigError::Syntax { line, .. }) = Provider::parse(text) else {
        panic!("expected a syntax error");
    };

    assert_eq!(line, 4);
}

#[test]
fn a_provider_lists_every_command_it_will_run() {
    let provider = cloud();

    let fields: Vec<&str> = provider.commands().into_iter().map(|(field, _)| field).collect();

    assert_eq!(fields, ["create", "connect", "status"]);
    assert_eq!(provider.reach(), Reach::Connect);
    assert!(provider.has(Action::Status));
    assert!(!provider.has(Action::Delete));
}

#[test]
fn params_without_a_default_must_be_filled_and_undeclared_ones_are_refused() {
    let provider = cloud();
    let machine = name("box-1");

    let missing = provider.environment(&machine, &BTreeMap::new());
    let undeclared = provider.environment(
        &machine,
        &BTreeMap::from([
            ("region".to_owned(), "fsn1".to_owned()),
            ("zone".to_owned(), "a".to_owned()),
        ]),
    );

    assert!(matches!(missing, Err(ValuesError::Missing { ref param, .. }) if param == "region"));
    assert!(matches!(undeclared, Err(ValuesError::Undeclared { ref param, .. }) if param == "zone"));
}

#[test]
fn a_command_gets_its_name_params_and_host_variables_and_the_secret_is_not_shown() {
    let provider = cloud();
    let values = BTreeMap::from([("region".to_owned(), "fsn1 'eu'".to_owned())]);
    let invocation = provider
        .invocation(Action::Create, &name("box-1"), &values)
        .unwrap()
        .unwrap();
    let variables = Variables(BTreeMap::from([("CLOUD_TOKEN", "s3cr3t-value")]));

    let output = SystemRunner::new(&variables).run(&invocation).unwrap();

    assert_eq!(output.exit, Exit::Code(0));
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(stdout, "box-1|box-1|small|fsn1 'eu'|12\n");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(!stderr.contains("s3cr3t-value"));
    assert!(stderr.contains("CLOUD_TOKEN"));
}

#[test]
fn a_command_doesnt_see_this_processs_other_variables() {
    let provider = Provider::parse(
        "[providers.a]\ncreate = \"env\"\nconnect = \"x {cmd}\"\n",
    )
    .unwrap()
    .pop()
    .unwrap();
    let invocation = provider
        .invocation(Action::Create, &name("box-1"), &BTreeMap::new())
        .unwrap()
        .unwrap();
    let baseline = [
        "HOME", "PATH", "USER", "LOGNAME", "SHELL", "LANG", "LC_ALL", "TMPDIR", "SSH_AUTH_SOCK",
        // Set by the shell itself.
        "PWD", "SHLVL", "OLDPWD", "_",
    ];
    let outside: Vec<String> = std::env::vars()
        .map(|(name, _)| name)
        .filter(|name| !baseline.contains(&name.as_str()))
        // macOS gives every process this one.
        .filter(|name| name != "__CF_USER_TEXT_ENCODING")
        .collect();

    let output = SystemRunner::new(&()).run(&invocation).unwrap();

    let seen: Vec<String> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .filter_map(|line| line.split_once('=').map(|(name, _)| name.to_owned()))
        .collect();
    assert!(seen.contains(&"HUMATA_NAME".to_owned()));
    for name in outside {
        assert!(!seen.contains(&name), "{name} reached the command");
    }
}

#[test]
fn a_missing_host_variable_stops_the_command_before_it_runs() {
    let invocation = cloud()
        .invocation(
            Action::Create,
            &name("box-1"),
            &BTreeMap::from([("region".to_owned(), "fsn1".to_owned())]),
        )
        .unwrap()
        .unwrap();

    let result = SystemRunner::new(&()).run(&invocation);

    assert!(matches!(result, Err(RunError::MissingHostVariable { ref name }) if name == "CLOUD_TOKEN"));
}

#[test]
fn status_reads_a_word_or_json() {
    assert_eq!(parse_status(b"Running\n"), MachineStatus::Running);
    assert_eq!(parse_status(b"banner\nStopped\n"), MachineStatus::Stopped);
    assert_eq!(parse_status(br#"{"state":"terminated"}"#), MachineStatus::Stopped);
    assert!(matches!(parse_status(b"Broken"), MachineStatus::Other(_)));
}

#[test]
fn the_lima_provider_only_names_humata_vms() {
    let lima = Provider::lima().unwrap();

    for (field, command) in lima.commands() {
        if command.contains("limactl") {
            assert!(command.contains("humata2-{name}"), "{field}: {command}");
        }
    }
}
