//! Offline provider authoring CLI commands (`humata provider
//! {guide,schema,example,check,render,parse,request-import}`, PROVIDERCLI D906).

mod support;

use humata2_cli::{exit, rpc};
use humata2_proto::humata::v1 as pb;
use support::{FakeConsole, FakeDaemon, run};

#[tokio::test]
async fn guide_and_schema_run_offline_and_never_call_the_daemon() {
    let daemon = FakeDaemon::new();

    let console = FakeConsole::pipe();
    let code = run(&daemon, &console, &["provider", "guide"]).await;
    assert_eq!(code, exit::OK);
    assert!(console.stdout().contains("[providers.<id>]"));
    assert!(console.stdout().contains("request-import"));
    assert!(daemon.all_calls().is_empty());

    let console = FakeConsole::pipe();
    let code = run(&daemon, &console, &["provider", "schema", "--json"]).await;
    assert_eq!(code, exit::OK, "{}", console.stdout());
    let json = console.json();
    assert_eq!(json["ok"], true);
    assert_eq!(json["operation"], "provider.schema");
    assert!(json["result"]["provider_fields"].is_array());
    assert!(json["result"]["identity_fields"].is_array());
    assert!(json["result"]["discover_fields"].is_array());
    assert!(daemon.all_calls().is_empty());
}

#[tokio::test]
async fn every_example_prints_valid_toml_that_passes_check() {
    let daemon = FakeDaemon::new();

    // Omitting <kind> lists the available kinds.
    let list_console = FakeConsole::pipe();
    let code = run(&daemon, &list_console, &["provider", "example", "--json"]).await;
    assert_eq!(code, exit::OK);
    let kinds: Vec<String> = list_console.json()["result"]["examples"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["kind"].as_str().unwrap().to_owned())
        .collect();
    assert!(kinds.contains(&"lima".to_owned()));
    assert!(kinds.contains(&"cloud".to_owned()));
    assert!(kinds.contains(&"terraform".to_owned()));
    assert!(kinds.contains(&"ssh".to_owned()));

    for kind in &kinds {
        let ex_console = FakeConsole::pipe();
        let code = run(&daemon, &ex_console, &["provider", "example", kind]).await;
        assert_eq!(code, exit::OK, "example {kind} failed");
        let toml = ex_console.stdout();

        let check_console = FakeConsole::pipe().with_stdin(toml.as_bytes());
        let code = run(&daemon, &check_console, &["--json", "provider", "check", "-"]).await;
        assert_eq!(code, exit::OK, "example {kind} failed check: {}", check_console.stdout());
        assert_eq!(check_console.json()["result"]["ok"], true);
    }

    assert!(daemon.all_calls().is_empty());
}

#[tokio::test]
async fn check_reports_actionable_field_errors_for_invalid_provider_files() {
    let daemon = FakeDaemon::new();
    let dir = tempfile::tempdir().unwrap();
    let bad = dir.path().join("bad.toml");
    std::fs::write(
        &bad,
        "[providers.broken]\nconnect = \"ssh {name}\"\n",
    )
    .unwrap();

    let console = FakeConsole::pipe();
    let path = bad.display().to_string();
    let code = run(&daemon, &console, &["--json", "provider", "check", &path]).await;

    assert_eq!(code, exit::USAGE);
    let json = console.json();
    assert_eq!(json["ok"], false);
    assert_eq!(json["error"]["kind"], "invalid_field");
    let problems = json["error"]["details"]["problems"].as_array().unwrap();
    assert_eq!(problems[0]["field"], "connect");
    assert!(daemon.all_calls().is_empty());
}

#[tokio::test]
async fn render_prints_exact_argv_and_env_without_running_anything() {
    let daemon = FakeDaemon::new();
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("cloud.toml");
    std::fs::write(
        &file,
        r#"
[providers.mycloud]
label = "My Cloud"
params = [{ name = "region", default = "us-east-1" }]
env = ["CLOUD_TOKEN"]
create = "mycloud create {name} --region \"$HUMATA_REGION\""
connect = "mycloud ssh {name} -- {cmd}"
run_on = "bastion"
"#,
    )
    .unwrap();

    let console = FakeConsole::pipe();
    let path = file.display().to_string();
    let code = run(
        &daemon,
        &console,
        &[
            "--json",
            "provider",
            "render",
            &path,
            "create",
            "--name",
            "dev-1",
            "--param",
            "region=eu-west-1",
        ],
    )
    .await;

    assert_eq!(code, exit::OK, "{}", console.stdout());
    let result = &console.json()["result"];
    assert_eq!(result["provider"], "mycloud");
    assert_eq!(result["action"], "create");
    assert_eq!(result["machine"], "dev-1");
    assert_eq!(result["run_on"], "bastion");
    assert_eq!(result["program"], "/bin/sh");
    assert_eq!(result["args"][0], "-c");
    assert_eq!(
        result["args"][1],
        "mycloud create dev-1 --region \"$HUMATA_REGION\""
    );
    let env = result["env"].as_array().unwrap();
    assert!(env.iter().any(|v| v["name"] == "HUMATA_NAME" && v["value"] == "dev-1"));
    assert!(env.iter().any(|v| v["name"] == "HUMATA_REGION" && v["value"] == "eu-west-1"));
    assert!(env.iter().any(|v| v["name"] == "CLOUD_TOKEN" && v["source"] == "host_variable"));
    assert!(daemon.all_calls().is_empty());
}

#[tokio::test]
async fn parse_validates_identity_status_and_discover_outputs_offline() {
    let daemon = FakeDaemon::new();

    // Valid identity via stdin
    let ok_id = br#"{"name":"box-1","ssh":{"host":"10.0.0.2","user":"ubuntu"},"os":"linux","arch":"x86_64"}"#;
    let console = FakeConsole::pipe().with_stdin(ok_id);
    let code = run(
        &daemon,
        &console,
        &["--json", "provider", "parse", "identity", "-", "--name", "box-1"],
    )
    .await;
    assert_eq!(code, exit::OK, "{}", console.stdout());
    assert_eq!(console.json()["result"]["kind"], "identity");
    assert_eq!(console.json()["result"]["os"], "linux");

    // Bad status JSON fails with exit 2 and names the missing field
    let bad_status = br#"{"instance_id":"i-1"}"#;
    let console = FakeConsole::pipe().with_stdin(bad_status);
    let code = run(&daemon, &console, &["--json", "provider", "parse", "status", "-"]).await;
    assert_eq!(code, exit::USAGE);
    assert_eq!(console.json()["error"]["details"]["field"], "state");

    // Valid discover array
    let ok_disc = br#"[{"name":"web-1","state":"running"}]"#;
    let console = FakeConsole::pipe().with_stdin(ok_disc);
    let code = run(&daemon, &console, &["--json", "provider", "parse", "discover", "-"]).await;
    assert_eq!(code, exit::OK, "{}", console.stdout());
    assert_eq!(console.json()["result"]["instances"][0]["name"], "web-1");
    assert!(daemon.all_calls().is_empty());
}

#[tokio::test]
async fn request_import_sends_request_provider_import_rpc() {
    let daemon = FakeDaemon::new();
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("corp.toml");
    let body = "[providers.corp]\ncreate = \"corp mk {name}\"\nconnect = \"corp sh {name} {cmd}\"\n";
    std::fs::write(&file, body).unwrap();

    daemon.refuse(
        rpc::provider::REQUEST_PROVIDER_IMPORT,
        pb::error::Kind::ApprovalPending(pb::ApprovalPending {
            request: Some(pb::RequestId {
                value: "rq_import_1".to_owned(),
            }),
            ..Default::default()
        }),
    );

    let console = FakeConsole::pipe();
    let path = file.display().to_string();
    let code = run(
        &daemon,
        &console,
        &[
            "--json",
            "provider",
            "request-import",
            &path,
            "--reason",
            "need corp VMs",
        ],
    )
    .await;

    assert_eq!(code, exit::PENDING, "{}", console.stdout());
    let sent: Vec<pb::RequestProviderImportRequest> =
        daemon.sent(rpc::provider::REQUEST_PROVIDER_IMPORT);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].file_name, "corp.toml");
    assert_eq!(sent[0].text, body);
    assert_eq!(sent[0].reason, "need corp VMs");
}
