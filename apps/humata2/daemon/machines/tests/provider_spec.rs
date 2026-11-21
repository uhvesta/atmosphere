//! Behavioural tests for the offline provider authoring engine
//! (`humata2_machines::spec`, PROVIDERCLI D906): examples, schema fidelity,
//! offline check, exact render parity with the real `Provider`/`Link` engine,
//! and output parsing (`identity`, `status`, `discover`).

use std::collections::BTreeMap;

use humata2_machines::spec::{
    check_providers, list_examples, parse_provider_output, provider_example, provider_guide,
    provider_schema, render_provider_action, ParseKind, ParsedOutput, RenderAction, RenderError,
};
use humata2_machines::{EnvValue, MachineName, MachineStatus, Provider};

#[test]
fn every_built_in_example_passes_check_and_real_parser() {
    let examples = list_examples();
    assert!(examples.len() >= 4, "expected lima, cloud, terraform, ssh");

    for example in examples {
        let report = check_providers(example.toml);
        assert!(
            report.ok,
            "example `{}` failed check: {:?}",
            example.kind, report.problems
        );
        assert!(
            !report.providers.is_empty(),
            "example `{}` defined no providers",
            example.kind
        );
        let parsed = Provider::parse(example.toml).expect("real parser accepts example");
        assert_eq!(parsed.len(), report.providers.len());
    }

    // Aliases resolve to their canonical examples.
    assert_eq!(provider_example("lima").unwrap().kind, "lima");
    assert_eq!(provider_example("cloud-cli").unwrap().kind, "cloud");
    assert_eq!(provider_example("tf").unwrap().kind, "terraform");
    assert_eq!(provider_example("ssh-fleet").unwrap().kind, "ssh");
    assert!(provider_example("nonexistent").is_err());
}

#[test]
fn guide_covers_core_provider_concepts() {
    let guide = provider_guide();
    for needle in [
        "[providers.<id>]",
        "connect",
        "identity",
        "list",
        "{name}",
        "{cmd}",
        "HUMATA_NAME",
        "run_on",
        "interactive",
        "known_hosts",
        "request-import",
    ] {
        assert!(guide.contains(needle), "guide missing `{needle}`");
    }
}

#[test]
fn schema_fields_match_what_the_real_parsers_accept() {
    let schema = provider_schema();

    // Every provider field in schema is accepted by Provider::parse.
    let mut toml = String::from("[providers.all-fields]\n");
    for field in &schema.provider_fields {
        match field.name {
            "label" => toml.push_str("label = \"All Fields\"\n"),
            "connect" => toml.push_str("connect = \"ssh {name} -- {cmd}\"\n"),
            "wake_on_demand" => toml.push_str("wake_on_demand = true\n"),
            "interactive" => toml.push_str("interactive = false\n"),
            "params" => {
                toml.push_str("params = [{ name = \"size\", prompt = \"Size\", default = \"sm\" }]\n")
            }
            "env" => toml.push_str("env = [\"MY_TOKEN\"]\n"),
            "run_on" => toml.push_str("run_on = \"build-host\"\n"),
            other => toml.push_str(&format!("{other} = \"echo {name}\"\n", name = "{name}")),
        }
    }
    let report = check_providers(&toml);
    assert!(
        report.ok,
        "provider using all schema fields should pass: {:?}",
        report.problems
    );

    // Every documented status word parses to its canonical status.
    for group in &schema.status_words {
        for word in &group.words {
            let parsed = parse_provider_output(ParseKind::Status, word.as_bytes(), None)
                .expect("documented status word parses");
            let ParsedOutput::Status { state, .. } = parsed else {
                panic!("expected Status output");
            };
            assert_eq!(
                state, group.canonical,
                "word `{word}` expected `{}`",
                group.canonical
            );
        }
    }

    // Schema text rendering is non-empty and mentions every section.
    let text = schema.render_text();
    assert!(text.contains("Provider fields"));
    assert!(text.contains("Identity JSON output"));
    assert!(text.contains("Discover output"));
}

#[test]
fn check_rejects_invalid_toml_empty_files_and_bad_fields_with_actionable_errors() {
    // Empty file
    let empty = check_providers("");
    assert!(!empty.ok);
    assert_eq!(empty.problems[0].field, "providers");
    assert!(!empty.problems[0].fix.is_empty());

    // Syntax error / unknown key
    let unknown = check_providers(
        "[providers.a]\nconnect = \"ssh {name} {cmd}\"\nnot_a_real_key = \"x\"\n",
    );
    assert!(!unknown.ok);
    assert_eq!(unknown.problems[0].line, Some(3));

    // Multiple semantic problems reported together
    let broken = check_providers(
        r#"
[providers.Bad_Name]
start = "echo {cmd}"
params = [{ name = "name" }, { name = "BadParam" }]
env = ["PATH", "bad_var"]
"#,
    );
    assert!(!broken.ok);
    let fields: Vec<&str> = broken.problems.iter().map(|p| p.field.as_str()).collect();
    assert!(fields.contains(&"connect"), "missing link: {fields:?}");
    assert!(fields.contains(&"start"), "cmd outside connect: {fields:?}");
    assert!(fields.contains(&"params[0].name"), "reserved param: {fields:?}");
    assert!(fields.contains(&"env[0]"), "reserved env: {fields:?}");
    for p in &broken.problems {
        assert!(!p.fix.is_empty(), "every problem has a fix: {p:?}");
    }
}

#[test]
fn render_matches_daemon_invocation_for_actions_list_and_connect() {
    let cloud = provider_example("cloud").unwrap().toml;
    let parsed = Provider::parse(cloud).unwrap().pop().unwrap();
    let machine = MachineName::new("dev-box-1").unwrap();
    let mut params = BTreeMap::new();
    params.insert("region".to_owned(), "eu-west-1".to_owned());

    // Create action matches Provider::invocation exactly.
    let real_create = parsed
        .invocation(humata2_machines::Action::Create, &machine, &params)
        .unwrap()
        .unwrap();
    let rendered_create = render_provider_action(
        cloud,
        None,
        RenderAction::Create,
        "dev-box-1",
        &params,
        None,
    )
    .unwrap();
    assert_eq!(rendered_create.program, real_create.program);
    assert_eq!(rendered_create.args, real_create.args);
    assert_eq!(
        rendered_create.timeout_seconds,
        real_create.timeout.map(|d| d.as_secs())
    );
    assert_eq!(rendered_create.env.len(), real_create.env.len());
    for (rendered_var, (real_name, real_val)) in rendered_create.env.iter().zip(real_create.env.iter())
    {
        assert_eq!(&rendered_var.name, real_name);
        match real_val {
            EnvValue::Value(v) => {
                assert_eq!(rendered_var.value.as_deref(), Some(v.as_str()));
                assert_eq!(rendered_var.source, "humata");
            }
            EnvValue::HostVariable(_) => {
                assert_eq!(rendered_var.value, None);
                assert_eq!(rendered_var.source, "host_variable");
            }
        }
    }

    // List/discover action matches Provider::list_invocation exactly.
    let real_list = parsed.list_invocation().unwrap();
    let rendered_list = render_provider_action(
        cloud,
        None,
        RenderAction::List,
        "dev-box-1",
        &BTreeMap::new(),
        None,
    )
    .unwrap();
    assert_eq!(rendered_list.program, real_list.program);
    assert_eq!(rendered_list.args, real_list.args);

    // Connect action quotes remote command identically to Link.
    let rendered_connect = render_provider_action(
        cloud,
        None,
        RenderAction::Connect,
        "dev-box-1",
        &params,
        Some("echo 'hello world'"),
    )
    .unwrap();
    assert_eq!(rendered_connect.program, "/bin/sh");
    assert_eq!(rendered_connect.args[0], "-c");
    assert!(rendered_connect.args[1].contains("cloudctl vm ssh dev-box-1"));
    assert!(rendered_connect.args[1].contains("hello world"));
}

#[test]
fn render_reports_missing_param_undefined_action_and_ambiguous_provider() {
    let tf = provider_example("terraform").unwrap().toml;
    // Missing required param `workspace_dir`
    let err = render_provider_action(
        tf,
        None,
        RenderAction::Create,
        "box-1",
        &BTreeMap::new(),
        None,
    )
    .unwrap_err();
    assert!(matches!(err, RenderError::BadValues(_)), "{err:?}");

    // Undefined action (`connect` on terraform which uses `identity`)
    let mut params = BTreeMap::new();
    params.insert("workspace_dir".to_owned(), "/tmp/tf".to_owned());
    let err = render_provider_action(tf, None, RenderAction::Connect, "box-1", &params, None)
        .unwrap_err();
    assert!(
        matches!(err, RenderError::ActionUndefined { action: "connect", .. }),
        "{err:?}"
    );

    // Ambiguous provider without --provider
    let multi = "[providers.a]\nconnect = \"a {name} {cmd}\"\n[providers.b]\nconnect = \"b {name} {cmd}\"\n";
    let err = render_provider_action(
        multi,
        None,
        RenderAction::Connect,
        "box-1",
        &BTreeMap::new(),
        None,
    )
    .unwrap_err();
    assert!(matches!(err, RenderError::AmbiguousProvider { .. }), "{err:?}");
    let ok = render_provider_action(
        multi,
        Some("b"),
        RenderAction::Connect,
        "box-1",
        &BTreeMap::new(),
        None,
    )
    .unwrap();
    assert_eq!(ok.provider, "b");
}

#[test]
fn parse_validates_identity_status_and_discover_and_rejects_bad_input() {
    // Valid identity
    let id_json = br#"{"name":"dev-1","ssh":{"host":"10.0.0.5","port":2222,"user":"ubuntu","identity_file":"/home/u/.ssh/id_ed25519"},"os":"linux","arch":"arm64","state":"running"}"#;
    let parsed = parse_provider_output(ParseKind::Identity, id_json, Some("dev-1")).unwrap();
    let ParsedOutput::Identity {
        name,
        os,
        arch,
        ssh,
        state,
        ..
    } = parsed
    else {
        panic!("expected identity");
    };
    assert_eq!(name, "dev-1");
    assert_eq!(os, "linux");
    assert_eq!(arch, "aarch64");
    assert_eq!(ssh.port, Some(2222));
    assert_eq!(state.as_deref(), Some("running"));

    // Identity with wrong name
    let err = parse_provider_output(ParseKind::Identity, id_json, Some("other-box")).unwrap_err();
    assert_eq!(err.field.as_deref(), Some("name"));

    // Identity containing private key material is refused
    let leaky = br#"{"name":"dev-1","ssh":{"host":"10.0.0.5","user":"u","identity_file":"-----BEGIN OPENSSH PRIVATE KEY-----"},"os":"linux","arch":"amd64"}"#;
    let err = parse_provider_output(ParseKind::Identity, leaky, Some("dev-1")).unwrap_err();
    assert!(err.message.contains("key material"), "{}", err.message);

    // Status JSON and plain text
    let st_json = br#"{"state":"suspended","instance_id":"i-123"}"#;
    let parsed_st = parse_provider_output(ParseKind::Status, st_json, None).unwrap();
    assert!(matches!(
        parsed_st,
        ParsedOutput::Status { ref state, ref instance_id, .. }
            if state == "sleeping" && instance_id.as_deref() == Some("i-123")
    ));
    let bad_st = parse_provider_output(ParseKind::Status, br#"{"instance_id":"i-123"}"#, None)
        .unwrap_err();
    assert_eq!(bad_st.field.as_deref(), Some("state"));

    // Discover JSON array
    let disc = br#"[{"name":"node-1","state":"up","params":{"zone":"us-east-1"}}]"#;
    let parsed_disc = parse_provider_output(ParseKind::Discover, disc, None).unwrap();
    let ParsedOutput::Discover { instances } = parsed_disc else {
        panic!("expected discover");
    };
    assert_eq!(instances.len(), 1);
    assert_eq!(instances[0].name, "node-1");
    assert_eq!(instances[0].state, "running");

    // Discover non-array rejected
    let bad_disc = parse_provider_output(ParseKind::Discover, br#"{"name":"node-1"}"#, None)
        .unwrap_err();
    assert_eq!(bad_disc.field.as_deref(), Some("output"));

    // Sanity: MachineStatus import check
    let _ = MachineStatus::Running;
}
